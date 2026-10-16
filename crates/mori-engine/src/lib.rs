//! Mori's Chromium engine.
//!
//! Mori renders every page offscreen through CEF (the Chromium Embedded
//! Framework): Chromium paints into BGRA buffers that the GPUI chrome draws,
//! and the chrome forwards mouse, keyboard, and IME input back into Chromium.
//!
//! Everything in this crate runs on the browser process main thread. CEF is
//! initialised with `external_message_pump`, so CEF callbacks only happen
//! from inside [`pump`], which the UI calls from its own event loop. That is
//! why the view state below uses `Rc`/`RefCell` rather than locks.

mod app;
mod client;
mod devtools;
mod downloads;
mod drag;
mod file_dialog;
pub mod keys;
mod print;
mod view;

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicI64, Ordering},
};

use anyhow::{Context as _, Result, bail};
use cef::{args::Args, *};

pub use devtools::Sink as DevToolsSink;
pub use downloads::{
    DownloadId, DownloadState, DownloadUpdate, cancel_download, is_controllable, pause_download,
    resume_download, take_download_updates, unique_path,
};
pub use view::{
    ContextMenuInfo, CursorKind, DialogId, DialogKind, Disposition, FileDialogId, FileDialogMode,
    FileDialogRequest, Frame, JsDialog, MouseButton, PermissionId, PermissionKind,
    PermissionRequest, PopupFeatures, PopupFrame, ViewEvent, ViewId, WebView, take_dirty_views,
};

/// Configuration for [`initialize`].
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Root directory for all Chromium profile data (cookies, cache, storage).
    pub profile_dir: PathBuf,
    /// Directory downloads are saved into.
    pub downloads_dir: PathBuf,
    /// Product token appended to the default Chromium user agent.
    pub user_agent_product: Option<String>,
    /// Optional Chrome DevTools Protocol port for external tooling.
    pub remote_debugging_port: Option<u16>,
    /// UI locale, e.g. `en-US`.
    pub locale: String,
    /// Disable Chromium's sandbox (needed on systems without user namespaces).
    pub no_sandbox: bool,
}

static INITIALIZED: AtomicBool = AtomicBool::new(false);
/// Command-line URLs from relaunches of the app, waiting for the UI.
static RELAUNCHES: std::sync::Mutex<Vec<Vec<String>>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn push_relaunch(urls: Vec<String>) {
    if let Ok(mut q) = RELAUNCHES.lock() {
        q.push(urls);
    }
}

/// Relaunches since the last call; each entry is that launch's URL
/// arguments (possibly empty, meaning "just bring the window forward").
pub fn take_relaunches() -> Vec<Vec<String>> {
    RELAUNCHES.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}
/// Milliseconds since the UNIX epoch at which CEF asked to be pumped next,
/// or `i64::MAX` when it has not asked.
static NEXT_PUMP_AT: AtomicI64 = AtomicI64::new(0);

/// Must be the very first thing `main` calls.
///
/// Chromium launches renderer, GPU, and utility processes by re-executing the
/// current binary with a `--type=` switch. In those child processes this runs
/// the CEF subprocess logic and returns its exit code; in the browser process
/// it returns `None` and the caller continues starting the UI.
pub fn run_subprocess_if_needed() -> Option<i32> {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = Args::new();
    let cmd = args.as_cmd_line()?;
    let is_browser_process = cmd.has_switch(Some(&CefString::from("type"))) != 1;
    let mut app = app::build_app();
    let code = execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if is_browser_process {
        None
    } else {
        Some(code.max(0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitOutcome {
    /// CEF is running in this process.
    Started,
    /// Another Mori instance owns the profile and received our arguments.
    HandedOff,
}

/// Initialise CEF in the browser process.
pub fn initialize(config: &EngineConfig) -> Result<InitOutcome> {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        bail!("engine already initialised");
    }
    let resources = resources_dir().context("locating CEF resources")?;
    std::fs::create_dir_all(&config.profile_dir).ok();
    std::fs::create_dir_all(&config.downloads_dir).ok();
    downloads::set_downloads_dir(config.downloads_dir.clone());

    let exe = std::env::current_exe().context("resolving current executable")?;
    let path_str = |p: &Path| CefString::from(p.to_string_lossy().as_ref());

    let settings = Settings {
        no_sandbox: config.no_sandbox as _,
        browser_subprocess_path: path_str(&exe),
        windowless_rendering_enabled: true as _,
        external_message_pump: true as _,
        multi_threaded_message_loop: false as _,
        root_cache_path: path_str(&config.profile_dir),
        cache_path: path_str(&config.profile_dir.join("Default")),
        persist_session_cookies: true as _,
        user_agent_product: config
            .user_agent_product
            .as_deref()
            .map(CefString::from)
            .unwrap_or_default(),
        locale: CefString::from(config.locale.as_str()),
        resources_dir_path: path_str(&resources),
        locales_dir_path: path_str(&resources.join("locales")),
        remote_debugging_port: config.remote_debugging_port.map(i32::from).unwrap_or(0),
        log_severity: LogSeverity::WARNING,
        background_color: 0xFFFFFFFF,
        ..Default::default()
    };

    let args = Args::new();
    let mut app = app::build_app();
    let ok = cef::initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if ok != 1 {
        // Another Mori is running with this profile; Chromium has already
        // forwarded our command line to it.
        if get_exit_code() == sys::cef_resultcode_t::CEF_RESULT_CODE_NORMAL_EXIT_PROCESS_NOTIFIED as i32 {
            return Ok(InitOutcome::HandedOff);
        }
        bail!("CEF failed to initialise (cef_initialize returned {ok})");
    }
    Ok(InitOutcome::Started)
}

/// Run one iteration of Chromium's browser-process message loop.
///
/// All CEF callbacks (paints, navigation state, popups, downloads, ...) are
/// delivered from inside this call.
pub fn pump() {
    if INITIALIZED.load(Ordering::Relaxed) {
        NEXT_PUMP_AT.store(i64::MAX, Ordering::Relaxed);
        do_message_loop_work();
    }
}

/// How long the UI may wait before calling [`pump`] again, in milliseconds,
/// clamped to `[min, max]`. CEF tells us when it has delayed work queued;
/// otherwise the caller's `max` keeps renderers and IPC responsive.
pub fn next_pump_delay_ms(min: u64, max: u64) -> u64 {
    let at = NEXT_PUMP_AT.load(Ordering::Relaxed);
    if at == i64::MAX {
        return max;
    }
    let now = now_ms();
    (at - now).clamp(min as i64, max as i64) as u64
}

pub(crate) fn schedule_pump(delay_ms: i64) {
    let at = now_ms().saturating_add(delay_ms.max(0));
    NEXT_PUMP_AT.fetch_min(at, Ordering::Relaxed);
    if let Some(wake) = PUMP_WAKER.get() {
        wake();
    }
}

type Waker = Box<dyn Fn() + Send + Sync>;
static PUMP_WAKER: std::sync::OnceLock<Waker> = std::sync::OnceLock::new();

/// Register a callback that wakes the UI's pump loop whenever CEF schedules
/// work. It may be called from any thread and must be cheap.
pub fn set_pump_waker(wake: impl Fn() + Send + Sync + 'static) {
    let _ = PUMP_WAKER.set(Box::new(wake));
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Shut CEF down. All views must already be closed (see [`close_all`]).
pub fn shutdown() {
    if INITIALIZED.swap(false, Ordering::SeqCst) {
        cef::shutdown();
    }
}

/// Ask every live browser to close, then pump until they are gone or the
/// timeout elapses. Call before [`shutdown`].
pub fn close_all(timeout: std::time::Duration) {
    view::close_all_views();
    let deadline = std::time::Instant::now() + timeout;
    while view::live_view_count() > 0 && std::time::Instant::now() < deadline {
        pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Locate the directory holding `libcef.so`, the `.pak` files, and `locales/`.
///
/// Packaged builds ship them next to the executable. Development builds use
/// the CEF distribution that `cef-dll-sys` downloaded at build time.
fn resources_dir() -> Result<PathBuf> {
    let exe_dir = std::env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    if exe_dir.join("icudtl.dat").exists() {
        return Ok(exe_dir);
    }
    if let Some(dir) = option_env!("MORI_CEF_DIR").map(PathBuf::from)
        && dir.join("icudtl.dat").exists()
    {
        return Ok(dir);
    }
    if let Ok(dir) = std::env::var("MORI_CEF_DIR").map(PathBuf::from)
        && dir.join("icudtl.dat").exists()
    {
        return Ok(dir);
    }
    bail!(
        "CEF resources not found next to {} or in MORI_CEF_DIR",
        exe_dir.display()
    )
}

/// The Chromium version CEF was built from, e.g. `154.0.8037.98`.
pub fn chromium_version() -> &'static str {
    let full = std::str::from_utf8(sys::CEF_VERSION)
        .unwrap_or("")
        .trim_end_matches('\0');
    full.rsplit("chromium-").next().unwrap_or(full)
}
