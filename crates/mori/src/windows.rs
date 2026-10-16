//! Browser windows: normal windows, private windows and Little Mori.
//!
//! Every window's root is a [`Browser`] holding its own copy of the stores
//! (workspace, settings, history, bookmarks, ...). When a window changes
//! (it notifies), the stores it shares with each other window are compared
//! and copied over, so all windows show the same spaces and tabs while each
//! keeps its own selection (`mori_core::windows`). What a window shares
//! depends on its kind ([`WindowKind::shares_with`]):
//!
//! - Normal windows share everything.
//! - Little Mori windows (links opened from other apps) have a workspace of
//!   their own with one page, and share everything else: what is browsed
//!   there is history like any other page.
//! - Private windows have a workspace of their own and share only
//!   preferences (settings, bookmarks, Air Traffic Control rules, Boosts).
//!   Their pages run in an in-memory browsing context and nothing they do
//!   is written to disk.
//!
//! One window writes the shared stores to disk: the first normal window, or
//! a Little Mori window while no normal window is open. A tab's running
//! page lives in one window at a time; selecting it in another window moves
//! the page there.
//!
//! Each window also has its own theme (its space's colours; a private
//! window is always dark), which is swapped into GPUI's globals before the
//! window draws.

use std::{cell::RefCell, collections::HashMap, sync::Arc};

use gpui_kit::{component::Theme as KitTheme, *};
use mori_core::{
    archive::ArchiveStore,
    bookmarks::BookmarkStore,
    boosts::BoostStore,
    content_blocker::SiteExceptions,
    downloads::DownloadStore,
    history::HistoryStore,
    navigation,
    omnibox::{self, Resolved},
    persist::Saver,
    profiles::{self, DeletedProfile, Profile, ProfileId, ProfileStore},
    routes::RouteStore,
    settings::{ExternalLinks, Settings},
    site_permissions::SitePermissionStore,
    windows::{SavedWindow, WINDOWS_FILE, WindowSelection, WindowSession, adopt_workspace, same_content},
    workspace::{SpaceId, TabId, TabRecord, Workspace},
    zoom::ZoomStore,
};
use mori_engine::{BrowsingContext, DownloadId, DownloadUpdate, Relaunch, WebView};

use crate::{
    actions::{NewPrivateWindow, NewWindow},
    browser::{Browser, ToastAction, ToastStyle},
    engine_hub::EngineHub,
    favicons::Favicons,
    theme::Tokens,
};

pub use mori_core::windows::WindowKind;

/// How a new window picks its first page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// Show the selected tab of the window's selection.
    Restore,
    /// Show the space's new-tab page with the command bar (Ctrl+N).
    Blank,
}

/// Where the running page of a tab is, for [`take_page_from_other_window`].
pub enum PageElsewhere {
    /// No other window has it loaded.
    Nowhere,
    /// Another window had it loaded and handed it over, with what that
    /// window knew of its media.
    Moved(WebView, Option<crate::ui::media::TabMedia>),
    /// Another window has it, waiting for the user; it stays there.
    Busy,
}

/// Everything a new window's [`Browser`] starts from.
pub struct Seed {
    pub kind: WindowKind,
    pub ws: Workspace,
    pub settings: Settings,
    pub history: HistoryStore,
    pub bookmarks: BookmarkStore,
    pub archive: ArchiveStore,
    pub routes: RouteStore,
    pub boosts: BoostStore,
    pub site_permissions: SitePermissionStore,
    pub zoom: ZoomStore,
    pub downloads: DownloadStore,
    pub favicons: Favicons,
    pub private_context: Option<BrowsingContext>,
    pub ephemeral_session: bool,
    pub start: Start,
    /// Sites the content blocker is off for (one blocker serves every
    /// window, so every window shares them).
    pub blocker_exceptions: SiteExceptions,
}

impl Seed {
    /// The stores as saved on disk: the first window of a run.
    fn load() -> Seed {
        let dir = mori_core::paths::data_dir();
        let mut ws = Workspace::load(&dir);
        // `MORI_START_URL` sessions are throwaway (tests): one tab, and the
        // real session file is never overwritten.
        let start_url = std::env::var("MORI_START_URL").ok();
        if let Some(url) = &start_url {
            ws = Workspace::new();
            ws.add_new_tab(TabRecord::new(url.clone()));
        }
        Seed {
            kind: WindowKind::Normal,
            ws,
            settings: Settings::load(&dir),
            history: HistoryStore::load(&dir),
            bookmarks: BookmarkStore::load(&dir),
            archive: ArchiveStore::load(&dir),
            routes: RouteStore::load(&dir),
            boosts: BoostStore::load(&dir),
            site_permissions: SitePermissionStore::load(&dir),
            zoom: ZoomStore::load(&dir),
            downloads: DownloadStore::load(&dir),
            favicons: Favicons::new(dir.join("favicons")),
            private_context: None,
            ephemeral_session: start_url.is_some(),
            start: Start::Restore,
            blocker_exceptions: SiteExceptions::load(&dir),
        }
    }
}

/// Default and smallest window sizes per kind.
const NORMAL_SIZE: (f32, f32) = (1280., 820.);
const NORMAL_MIN: (f32, f32) = (640., 420.);
const LITTLE_SIZE: (f32, f32) = (900., 640.);
const LITTLE_MIN: (f32, f32) = (420., 320.);

/// Name of the one space of a Little Mori window.
const LITTLE_SPACE: &str = "Little Mori";
/// Name of the space of a private window.
const PRIVATE_SPACE: &str = "Private";

struct Entry {
    handle: AnyWindowHandle,
    browser: WeakEntity<Browser>,
    kind: WindowKind,
    /// When the window was last activated (registry clock).
    last_active: u64,
    /// Normal windows: what the window shows, for `windows.json`.
    saved: SavedWindow,
    /// "Space — page" for the "Move to window" menu.
    label: String,
    /// Little Mori: the tab the window was opened for.
    little_tab: Option<TabId>,
    _subscriptions: Vec<Subscription>,
}

struct CachedTheme {
    tokens: Tokens,
    kit: KitTheme,
    base: gpui_kit::base::Theme,
}

#[derive(Default)]
struct Registry {
    entries: Vec<Entry>,
    clock: u64,
    profiles: ProfileStore,
    /// Writes `windows.json` and `profiles.json`.
    saver: Option<Saver>,
    /// The last normal window that closed while other windows stayed open:
    /// the next normal window (or start) brings it back.
    last_closed: Option<SavedWindow>,
    themes: HashMap<WindowId, CachedTheme>,
    /// Whose theme is in GPUI's globals right now.
    theme_owner: Option<WindowId>,
    /// The window each running download reports to.
    downloads: HashMap<DownloadId, WindowId>,
    quitting: bool,
    /// The saved normal windows were brought back (or there were none to
    /// bring back). Until then a run that started with Little Mori or a
    /// private window must not overwrite `windows.json` with one window.
    restored: bool,
    /// A window is being created and isn't registered yet: pages it took
    /// from other windows don't show up in the registry until then.
    opening: bool,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

fn with<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    REGISTRY.with(|r| f(&mut r.borrow_mut()))
}

fn browser_of(id: WindowId) -> Option<(AnyWindowHandle, Entity<Browser>, WindowKind)> {
    with(|r| {
        r.entries
            .iter()
            .find(|e| e.handle.window_id() == id)
            .and_then(|e| Some((e.handle, e.browser.upgrade()?, e.kind)))
    })
}

/// Open windows of `kind` (any kind with `None`), most recently active first.
fn browsers(kind: Option<WindowKind>) -> Vec<(AnyWindowHandle, Entity<Browser>, WindowKind)> {
    with(|r| {
        let mut list: Vec<_> = r
            .entries
            .iter()
            .filter(|e| kind.is_none_or(|k| e.kind == k))
            .filter_map(|e| Some((e.last_active, e.handle, e.browser.upgrade()?, e.kind)))
            .collect();
        list.sort_by_key(|(at, ..)| std::cmp::Reverse(*at));
        list.into_iter().map(|(_, h, b, k)| (h, b, k)).collect()
    })
}

/// Run `f` on a window's browser with its window.
fn update_in<R>(
    handle: AnyWindowHandle,
    browser: &Entity<Browser>,
    cx: &mut App,
    f: impl FnOnce(&mut Browser, &mut Window, &mut Context<Browser>) -> R,
) -> Option<R> {
    handle
        .update(cx, |_, window, cx| browser.update(cx, |b, cx| f(b, window, cx)))
        .ok()
}

// ---- Start, relaunch, quit -------------------------------------------------

/// Remove the data of profiles deleted last run. Call once the engine runs
/// in this process (so no other Mori uses the profile directory) and before
/// any window opens a profile.
pub fn purge_deleted_profiles(data_dir: &std::path::Path, profile_root: &std::path::Path) {
    let mut store = ProfileStore::load(data_dir);
    let doomed = store.take_pending_removal();
    if doomed.is_empty() {
        return;
    }
    for dir in &doomed {
        let path = profile_root.join(dir);
        if path.exists()
            && let Err(e) = std::fs::remove_dir_all(&path)
        {
            log::warn!("removing deleted profile {}: {e}", path.display());
        }
    }
    if let Err(e) = mori_core::paths::write_json_atomic(&data_dir.join(profiles::PROFILES_FILE), &store) {
        log::error!("saving profiles: {e:#}");
    }
}

/// Install the window actions and the engine hooks. Call once at start.
pub fn init(cx: &mut App) {
    let dir = mori_core::paths::data_dir();
    with(|r| {
        r.profiles = ProfileStore::load(&dir);
        r.saver = Some(Saver::new(&dir));
    });
    cx.on_action(|_: &NewWindow, cx| {
        open_new_window(cx);
    });
    cx.on_action(|_: &NewPrivateWindow, cx| open_private_window(Vec::new(), cx));
    EngineHub::on_relaunch(cx, on_relaunch);
    EngineHub::on_downloads(cx, route_downloads);
    cx.on_window_closed(on_window_closed).detach();
    cx.on_app_quit(|cx| {
        remember_sizes(cx);
        save_session();
        with(|r| {
            // Windows closing from here on are not the user closing them.
            r.quitting = true;
            if let Some(saver) = &r.saver {
                saver.flush();
            }
        });
        async {}
    })
    .detach();
}

/// Open the first windows: the saved ones, or a private window
/// (`--private-window`), or Little Mori for links handed over by another
/// app.
pub fn start(urls: Vec<String>, private: bool, cx: &mut App) {
    if private {
        open_private_window(urls, cx);
        return;
    }
    let dir = mori_core::paths::data_dir();
    let little = !urls.is_empty()
        && std::env::var_os("MORI_START_URL").is_none()
        && Settings::load(&dir).external_links == ExternalLinks::LittleWindow;
    if little {
        for url in &urls {
            open_little(url, cx);
        }
        return;
    }
    restore(cx);
    if !urls.is_empty()
        && let Some((handle, browser, _)) = browsers(Some(WindowKind::Normal)).into_iter().next()
    {
        update_in(handle, &browser, cx, |b, window, cx| b.open_external_urls(&urls, window, cx));
    }
    offer_default_browser(cx);
}

/// Bring back the normal windows of the last session.
fn restore(cx: &mut App) {
    with(|r| r.restored = true);
    // From disk, or the stores of an open Little Mori window (with the
    // saved workspace).
    let first = seed_for(WindowKind::Normal, cx);
    let throwaway = first.ephemeral_session || std::env::var_os("MORI_START_URL").is_some();
    let saved = if throwaway {
        Vec::new()
    } else {
        // Spaces deleted last run (deleting is undoable until then) let go
        // of their profiles.
        with(|r| r.profiles.retain_spaces(|s| first.ws.space(s).is_some()));
        WindowSession::load(&mori_core::paths::data_dir()).windows
    };
    let saved = WindowSession { windows: saved }.restore(&first.ws);
    let mut first = Some(first);
    for window in saved {
        let mut seed = match first.take() {
            Some(seed) => seed,
            None => seed_for(WindowKind::Normal, cx),
        };
        window.selection.apply(&mut seed.ws);
        open(seed, window.size, cx);
    }
}

/// A later `mori` launch handed over its command line.
fn on_relaunch(relaunch: Relaunch, cx: &mut App) {
    let Relaunch { urls, new_window, private_window } = relaunch;
    if private_window {
        open_private_window(urls, cx);
        return;
    }
    if new_window {
        let browser = open_new_window(cx);
        if let Some((handle, browser)) = browser.filter(|_| !urls.is_empty()) {
            update_in(handle, &browser, cx, |b, window, cx| b.open_external_urls(&urls, window, cx));
        }
        return;
    }
    if urls.is_empty() {
        focus_normal_window(cx);
        return;
    }
    let links = browsers(None)
        .first()
        .map(|(_, b, _)| b.read(cx).settings.external_links)
        .unwrap_or_else(|| Settings::load(&mori_core::paths::data_dir()).external_links);
    match links {
        ExternalLinks::LittleWindow => {
            for url in &urls {
                open_little(url, cx);
            }
        }
        ExternalLinks::CurrentSpace => {
            if let Some((handle, browser)) = focus_normal_window(cx) {
                update_in(handle, &browser, cx, |b, window, cx| b.open_external_urls(&urls, window, cx));
            }
        }
    }
}

/// The most recently used normal window, brought to the front; one is
/// opened (with the last session's selection) if none is open.
fn focus_normal_window(cx: &mut App) -> Option<(AnyWindowHandle, Entity<Browser>)> {
    if let Some((handle, browser, _)) = browsers(Some(WindowKind::Normal)).into_iter().next() {
        handle.update(cx, |_, window, _| window.activate_window()).ok();
        return Some((handle, browser));
    }
    // The run started with Little Mori or a private window: Mori proper
    // comes back with all its windows.
    if !with(|r| r.restored) {
        restore(cx);
        let (handle, browser, _) = browsers(Some(WindowKind::Normal)).into_iter().next()?;
        return Some((handle, browser));
    }
    let mut seed = seed_for(WindowKind::Normal, cx);
    let (selection, size) = returning_window(&seed.ws);
    selection.apply(&mut seed.ws);
    let browser = open(seed, size, cx)?;
    let handle = browser.read(cx).window_handle;
    Some((handle, browser))
}

/// Selection and size for a normal window opened while none is open.
fn returning_window(ws: &Workspace) -> (WindowSelection, Option<(f32, f32)>) {
    let saved = with(|r| r.last_closed.clone())
        .or_else(|| WindowSession::load(&mori_core::paths::data_dir()).windows.into_iter().next());
    match saved {
        Some(w) => (w.selection, w.size),
        None => (WindowSelection::of(ws), None),
    }
}

/// Ctrl+N: a new normal window on the current space, starting on its
/// new-tab page.
pub fn open_new_window(cx: &mut App) -> Option<(AnyWindowHandle, Entity<Browser>)> {
    // The saved windows first, so the new one doesn't replace them.
    if !with(|r| r.restored) {
        restore(cx);
    }
    let current = active_normal_space(cx);
    let mut seed = seed_for(WindowKind::Normal, cx);
    match current {
        Some(space) => seed.ws.active_space = space,
        None => returning_window(&seed.ws).0.apply(&mut seed.ws),
    }
    seed.start = Start::Blank;
    let browser = open(seed, None, cx)?;
    let handle = browser.read(cx).window_handle;
    Some((handle, browser))
}

/// The space of the active normal window (or the most recent one).
fn active_normal_space(cx: &App) -> Option<SpaceId> {
    let active = cx.active_window().map(|w| w.window_id());
    let normals = browsers(Some(WindowKind::Normal));
    normals
        .iter()
        .find(|(h, ..)| Some(h.window_id()) == active)
        .or(normals.first())
        .map(|(_, b, _)| b.read(cx).ws.active_space)
}

/// Ctrl+Shift+N: a private window. Private windows open at the same time
/// share one in-memory context; it is gone once the last one closes.
pub fn open_private_window(urls: Vec<String>, cx: &mut App) {
    let context = browsers(Some(WindowKind::Private))
        .iter()
        .find_map(|(_, b, _)| b.read(cx).private_context.clone())
        .or_else(BrowsingContext::private);
    let Some(context) = context else {
        log::error!("Chromium refused to create a private browsing context");
        if let Some((handle, browser, _)) = browsers(None).into_iter().next() {
            update_in(handle, &browser, cx, |b, _, cx| {
                b.toast("Couldn't open a private window", ToastStyle::Error, None, cx)
            });
        }
        return;
    };
    let mut seed = seed_for(WindowKind::Private, cx);
    seed.private_context = Some(context);
    let Some(browser) = open(seed, None, cx) else { return };
    if !urls.is_empty() {
        let handle = browser.read(cx).window_handle;
        update_in(handle, &browser, cx, |b, window, cx| b.open_external_urls(&urls, window, cx));
    }
}

/// Little Mori for one link handed over by another app.
pub fn open_little(input: &str, cx: &mut App) {
    let mut seed = seed_for(WindowKind::Little, cx);
    let url = match omnibox::resolve(input, &seed.settings) {
        Resolved::Url(u) => navigation::typed_url(input, &u),
        Resolved::Search(u) => u,
    };
    let mut record = TabRecord::new(url.clone());
    record.title = omnibox::placeholder_title(&url);
    let tab = seed.ws.add_new_tab(record);
    seed.ws.select(tab);
    // The page opens in the profile of the space "Open in …" takes it to,
    // so it is signed in as there and moves over without reloading.
    seed.private_context = little_target_space(cx)
        .and_then(|space| with(|r| r.profiles.profile_of(space)))
        .and_then(|profile| BrowsingContext::persistent(&profiles::dir_name(profile)));
    let Some(browser) = open(seed, None, cx) else { return };
    let id = browser.read(cx).window_handle.window_id();
    with(|r| {
        if let Some(e) = r.entries.iter_mut().find(|e| e.handle.window_id() == id) {
            e.little_tab = Some(tab);
        }
    });
}

/// The seed of a new window of `kind`, sharing what that kind shares with
/// the windows already open (or loaded from disk when none is).
fn seed_for(kind: WindowKind, cx: &App) -> Seed {
    let dir = mori_core::paths::data_dir();
    let normal = browsers(Some(WindowKind::Normal)).into_iter().next().map(|(_, b, _)| b);
    let shared = normal
        .clone()
        .or_else(|| browsers(Some(WindowKind::Little)).into_iter().next().map(|(_, b, _)| b));
    let mut seed = match &shared {
        Some(b) => {
            let b = b.read(cx);
            Seed {
                kind,
                ws: b.ws.clone(),
                settings: b.settings.clone(),
                history: b.history.clone(),
                bookmarks: b.bookmarks.clone(),
                archive: b.archive.clone(),
                routes: b.routes.clone(),
                boosts: b.boosts.clone(),
                site_permissions: b.site_permissions.clone(),
                zoom: b.zoom.clone(),
                downloads: b.downloads.clone(),
                favicons: b.favicons.clone(),
                private_context: None,
                ephemeral_session: b.ephemeral_session,
                start: Start::Restore,
                blocker_exceptions: b.privacy.exceptions.clone(),
            }
        }
        None => Seed::load(),
    };
    seed.kind = kind;
    match kind {
        WindowKind::Normal => {
            // With only Little Mori open: its stores, and the saved session
            // rather than its one-page workspace.
            if normal.is_none() && shared.is_some() {
                seed.ws = Workspace::load(&dir);
                seed.ephemeral_session = false;
            }
        }
        WindowKind::Little => {
            seed.ws = Workspace::new();
            seed.ws.spaces[0].name = LITTLE_SPACE.into();
            seed.ephemeral_session = true;
        }
        WindowKind::Private => {
            // Preferences carry over; nothing the private window browses
            // comes from or goes to the other windows.
            seed.history = HistoryStore::default();
            seed.archive = ArchiveStore::default();
            seed.site_permissions = SitePermissionStore::default();
            seed.zoom = ZoomStore::default();
            seed.downloads = DownloadStore::default();
            seed.favicons = Favicons::in_memory(dir.join("favicons"));
            seed.ws = Workspace::new();
            seed.ws.spaces[0].name = PRIVATE_SPACE.into();
            seed.ws.spaces[0].symbol = "hat-glasses".into();
            seed.ws.spaces[0].theme = mori_core::windows::private_theme();
            seed.ephemeral_session = true;
        }
    }
    seed
}

/// Open a window for `seed`.
fn open(seed: Seed, size: Option<(f32, f32)>, cx: &mut App) -> Option<Entity<Browser>> {
    let kind = seed.kind;
    let ((width, height), (min_w, min_h)) = match kind {
        WindowKind::Little => (LITTLE_SIZE, LITTLE_MIN),
        _ => (NORMAL_SIZE, NORMAL_MIN),
    };
    let (width, height) = size.unwrap_or((width, height));
    let title = match kind {
        WindowKind::Normal => "Mori",
        WindowKind::Private => "Mori Private",
        WindowKind::Little => "Little Mori",
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(gpui_kit::size(px(width), px(height)), cx)),
        app_id: Some("mori".into()),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        window_decorations: Some(WindowDecorations::Client),
        window_min_size: Some(gpui_kit::size(px(min_w), px(min_h))),
        // A parented window floats over the browser in tiling compositors.
        kind: match kind {
            WindowKind::Little => gpui_kit::WindowKind::Floating,
            _ => gpui_kit::WindowKind::Normal,
        },
        ..Default::default()
    };
    let was_opening = with(|r| std::mem::replace(&mut r.opening, true));
    let opened = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Browser::new(window, cx, seed)));
    let (handle, browser) = match opened {
        Ok(opened) => opened,
        Err(e) => {
            with(|r| r.opening = was_opening);
            log::error!("opening a window: {e:#}");
            return None;
        }
    };
    register(handle, &browser, kind, cx);
    with(|r| r.opening = was_opening);
    Some(browser)
}

fn register(handle: AnyWindowHandle, browser: &Entity<Browser>, kind: WindowKind, cx: &mut App) {
    // The window is gone before its browser is released: closing its pages
    // must outlive the registry entry.
    cx.observe_release(browser, |b, cx| b.close_pages(cx)).detach();
    let mut subscriptions = vec![cx.observe(browser, on_browser_changed)];
    let activation = handle
        .update(cx, |_, window, cx| {
            browser.update(cx, |_, cx| {
                cx.observe_window_activation(window, |b, window, _| {
                    let id = b.window_handle.window_id();
                    let active = window.is_window_active();
                    let size = window.bounds().size;
                    with(|r| {
                        r.clock += 1;
                        let clock = r.clock;
                        if let Some(e) = r.entries.iter_mut().find(|e| e.handle.window_id() == id) {
                            if active {
                                e.last_active = clock;
                            }
                            if e.kind == WindowKind::Normal {
                                e.saved.size = Some((f32::from(size.width), f32::from(size.height)));
                            }
                        }
                    });
                })
            })
        })
        .ok();
    subscriptions.extend(activation);
    let saved = SavedWindow {
        selection: WindowSelection::of(&browser.read(cx).ws),
        size: None,
    };
    with(|r| {
        r.clock += 1;
        r.entries.push(Entry {
            handle,
            browser: browser.downgrade(),
            kind,
            last_active: r.clock,
            saved,
            label: String::new(),
            little_tab: None,
            _subscriptions: subscriptions,
        });
        if kind == WindowKind::Normal {
            r.last_closed = None;
        }
    });
    elect_writer(cx);
    save_session();
}

/// One window writes the shared stores: the first normal window, else the
/// first Little Mori window. Private windows never write.
fn elect_writer(cx: &mut App) {
    let entries: Vec<(WindowKind, Option<Entity<Browser>>)> =
        with(|r| r.entries.iter().map(|e| (e.kind, e.browser.upgrade())).collect());
    let kinds: Vec<WindowKind> = entries.iter().map(|(k, _)| *k).collect();
    let writer = WindowKind::writer(&kinds);
    for (i, (_, browser)) in entries.into_iter().enumerate() {
        let Some(browser) = browser else { continue };
        let writes = Some(i) == writer;
        if browser.read(cx).writes_state != writes {
            browser.update(cx, |b, cx| {
                b.writes_state = writes;
                if writes {
                    b.schedule_save(cx);
                }
            });
        }
    }
}

fn on_window_closed(cx: &mut App, id: WindowId) {
    let (removed, stale_images) = with(|r| {
        let Some(i) = r.entries.iter().position(|e| e.handle.window_id() == id) else {
            return (Vec::new(), Vec::new());
        };
        let entry = r.entries.remove(i);
        if entry.kind == WindowKind::Normal
            && !r.quitting
            && !r.entries.iter().any(|e| e.kind == WindowKind::Normal)
        {
            // The last normal window: it comes back next time.
            r.last_closed = Some(entry.saved.clone());
        }
        if r.theme_owner == Some(id) {
            r.theme_owner = None;
        }
        let images = r
            .themes
            .remove(&id)
            .map(|t| [t.tokens.wash, t.tokens.grain].into_iter().flatten().collect::<Vec<_>>())
            .unwrap_or_default();
        r.downloads.retain(|_, w| *w != id);
        (entry._subscriptions, images)
    });
    for image in stale_images {
        let current = cx.try_global::<Tokens>().is_some_and(|t| uses_image(t, &image));
        if !current && !theme_image_in_use(&image) {
            cx.drop_image(image, None);
        }
    }
    drop(removed);
    // Sites the private windows turned blocking off for go with the last
    // of them.
    if with(|r| !r.entries.iter().any(|e| e.kind == WindowKind::Private))
        && let Some(blocker) = crate::browser::privacy::content_blocker(cx)
    {
        blocker.set_private_exceptions(&Default::default());
    }
    elect_writer(cx);
    save_session();
    if cx.windows().is_empty() {
        cx.quit();
        return;
    }
    // The closed window's pages are gone; a window that pointed at one of
    // them ("Open in another window") shows it again.
    cx.defer(reclaim_pages);
}

/// Note every normal window's size, for `windows.json`.
fn remember_sizes(cx: &mut App) {
    for (handle, _, kind) in browsers(None) {
        if kind != WindowKind::Normal {
            continue;
        }
        let Ok(size) = handle.update(cx, |_, window, _| window.bounds().size) else { continue };
        let id = handle.window_id();
        with(|r| {
            if let Some(e) = r.entries.iter_mut().find(|e| e.handle.window_id() == id) {
                e.saved.size = Some((f32::from(size.width), f32::from(size.height)));
            }
        });
    }
}

/// Queue `windows.json` (and `profiles.json`) for writing if they changed.
fn save_session() {
    with(|r| {
        if r.quitting {
            return;
        }
        let mut windows: Vec<SavedWindow> = r
            .entries
            .iter()
            .filter(|e| e.kind == WindowKind::Normal)
            .map(|e| e.saved.clone())
            .collect();
        if windows.is_empty() {
            windows.extend(r.last_closed.clone());
        }
        // A throwaway `MORI_START_URL` run leaves the saved windows alone.
        let throwaway = std::env::var_os("MORI_START_URL").is_some();
        let Some(saver) = &mut r.saver else { return };
        if !windows.is_empty()
            && !throwaway
            && let Err(e) = saver.stage(WINDOWS_FILE, &WindowSession { windows })
        {
            log::error!("saving windows: {e:#}");
        }
        if let Err(e) = saver.stage(profiles::PROFILES_FILE, &r.profiles) {
            log::error!("saving profiles: {e:#}");
        }
    });
}

// ---- Keeping windows in step ---------------------------------------------------

/// Stores one window hands to another; `None` where nothing changed.
#[derive(Default)]
struct Shared {
    ws: Option<Workspace>,
    settings: Option<Settings>,
    bookmarks: Option<BookmarkStore>,
    routes: Option<RouteStore>,
    boosts: Option<BoostStore>,
    history: Option<HistoryStore>,
    archive: Option<ArchiveStore>,
    zoom: Option<ZoomStore>,
    site_permissions: Option<SitePermissionStore>,
    downloads: Option<DownloadStore>,
    blocker_exceptions: Option<SiteExceptions>,
}

impl Shared {
    fn is_empty(&self) -> bool {
        self.ws.is_none()
            && self.settings.is_none()
            && self.bookmarks.is_none()
            && self.routes.is_none()
            && self.boosts.is_none()
            && self.history.is_none()
            && self.archive.is_none()
            && self.zoom.is_none()
            && self.site_permissions.is_none()
            && self.downloads.is_none()
            && self.blocker_exceptions.is_none()
    }

    /// What `to` lacks of `from`'s state.
    fn between(from: &Browser, to: &Browser) -> Shared {
        let mut out = Shared::default();
        let sharing = from.kind.shares_with(to.kind);
        if sharing.workspace && !same_content(&from.ws, &to.ws) {
            out.ws = Some(from.ws.clone());
        }
        macro_rules! copy_if_changed {
            ($($field:ident),*) => {$(
                if from.$field != to.$field {
                    out.$field = Some(from.$field.clone());
                }
            )*};
        }
        if sharing.preferences {
            // The sidebar's visibility is per window.
            let mut settings = from.settings.clone();
            settings.sidebar_visible = to.settings.sidebar_visible;
            if settings != to.settings {
                out.settings = Some(settings);
            }
            copy_if_changed!(bookmarks, routes, boosts);
            if from.privacy.exceptions != to.privacy.exceptions {
                out.blocker_exceptions = Some(from.privacy.exceptions.clone());
            }
        }
        if sharing.browsing_data {
            copy_if_changed!(history, archive, zoom, site_permissions, downloads);
        }
        out
    }
}

fn on_browser_changed(source: Entity<Browser>, cx: &mut App) {
    let (id, kind, handle) = {
        let b = source.read(cx);
        (b.window_handle.window_id(), b.kind, b.window_handle)
    };
    match kind {
        WindowKind::Little => tidy_little_window(&source, cx),
        WindowKind::Normal => {
            let (selection, label) = {
                let b = source.read(cx);
                (WindowSelection::of(&b.ws), window_label(b))
            };
            with(|r| {
                if let Some(e) = r.entries.iter_mut().find(|e| e.handle.window_id() == id) {
                    e.saved.selection = selection;
                    e.label = label;
                }
            });
            save_session();
            update_in(handle, &source, cx, |b, window, cx| b.fix_page_contexts(window, cx));
        }
        WindowKind::Private => {}
    }
    for (handle, target, _) in browsers(None) {
        if target == source {
            continue;
        }
        let shared = Shared::between(source.read(cx), target.read(cx));
        if shared.is_empty() {
            continue;
        }
        update_in(handle, &target, cx, |b, window, cx| b.take_shared(shared, window, cx));
    }
    reclaim_pages(cx);
}

/// Normal windows showing a tab whose page no other window shows any more
/// (it closed, moved on to another tab, or slept or reloaded the page)
/// load it again, rather than showing an empty card or a stale "Open in
/// another window".
fn reclaim_pages(cx: &mut App) {
    if with(|r| r.quitting || r.opening) {
        return;
    }
    for (handle, browser, _) in browsers(Some(WindowKind::Normal)) {
        let orphans: Vec<TabId> = {
            let b = browser.read(cx);
            [b.selected(), b.split]
                .into_iter()
                .flatten()
                .filter(|t| !b.live.contains_key(t))
                .filter(|t| b.ws.tab(*t).is_some_and(|r| !Browser::is_internal(&r.url)))
                .filter(|t| mori_core::windows::takes_page(false, page_elsewhere(b, *t, cx)))
                .collect()
        };
        if orphans.is_empty() {
            continue;
        }
        update_in(handle, &browser, cx, |b, window, cx| {
            let mut changed = false;
            for tab in orphans {
                b.realize(tab, window, cx);
                changed |= b.live.contains_key(&tab);
            }
            // (A page waiting for an answer elsewhere stays there; no
            // notify then, or this would run again and again.)
            if changed {
                cx.notify();
            }
        });
    }
}

/// "Work — GitHub" for a window showing GitHub in the Work space.
fn window_label(b: &Browser) -> String {
    let space = b.ws.active().name.clone();
    let page = b
        .selected()
        .and_then(|t| b.ws.tab(t))
        .map(|t| t.display_title().chars().take(32).collect::<String>())
        .filter(|t| !t.is_empty());
    match page {
        Some(page) => format!("{space} — {page}"),
        None => space,
    }
}

impl Browser {
    /// Another window changed stores this one shares.
    fn take_shared(&mut self, shared: Shared, window: &mut Window, cx: &mut Context<Self>) {
        let Shared {
            ws,
            settings,
            bookmarks,
            routes,
            boosts,
            history,
            archive,
            zoom,
            site_permissions,
            downloads,
            blocker_exceptions,
        } = shared;
        let mut retheme = false;
        if let Some(settings) = settings {
            retheme |= settings.appearance != self.settings.appearance;
            self.settings = settings;
        }
        macro_rules! take {
            ($($field:ident),*) => {$(
                if let Some(value) = $field {
                    self.$field = value;
                }
            )*};
        }
        take!(bookmarks, routes, boosts, history, archive, zoom, site_permissions, downloads);
        if let Some(exceptions) = blocker_exceptions {
            self.privacy.exceptions = exceptions;
        }
        if let Some(ws) = ws {
            let selected = self.selected();
            let space = self.ws.active_space;
            let theme = self.ws.active().theme.clone();
            self.ws = adopt_workspace(&self.ws, &ws);
            retheme |= self.ws.active_space != space || self.ws.active().theme != theme;
            self.after_workspace_change(selected, space, window, cx);
        }
        if retheme {
            self.apply_theme(Some(window), cx);
        }
        self.schedule_save(cx);
    }

    /// The workspace changed in another window: drop pages of tabs that
    /// are gone and show something else if this window's tab went away.
    fn after_workspace_change(
        &mut self,
        selected: Option<TabId>,
        space: SpaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let gone: Vec<TabId> = self.live.keys().copied().filter(|t| self.ws.tab(*t).is_none()).collect();
        for tab in gone {
            self.unload(tab, cx);
        }
        self.multi_select.retain(|t| self.ws.tab(*t).is_some());
        if self.split.is_some_and(|s| self.ws.split_of(s).is_none()) {
            self.close_split(cx);
        }
        if self.selected() == selected && self.ws.active_space == space {
            return;
        }
        let next = self.selected().or_else(|| self.ws.active().visible().first().copied());
        match next {
            // Another window shows it: no stealing, this one shows where it is.
            Some(tab) if shown_in_other_window(self, tab, cx) => {
                self.ws.select(tab);
                // The page that had the keys is gone from this window.
                self.focus_selected_page(window, cx);
                cx.notify();
            }
            Some(tab) => self.select_tab(tab, window, cx),
            None => {
                self.open_blank_tab(window, cx);
            }
        }
    }

    /// Reload pages whose space now uses another profile in that profile.
    fn fix_page_contexts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind != WindowKind::Normal {
            return;
        }
        let stale: Vec<TabId> = self
            .live
            .iter()
            .filter(|(tab, live)| live.view.in_context(&context_for_tab(self, **tab)) == Some(false))
            .map(|(tab, _)| *tab)
            .collect();
        if stale.is_empty() {
            return;
        }
        let (selected, split) = (self.selected(), self.split);
        for tab in &stale {
            self.unload(*tab, cx);
        }
        // Pages on screen load again right away (without leaving whatever
        // covers them, like the settings).
        for tab in [selected, split].into_iter().flatten().filter(|t| stale.contains(t)) {
            self.realize(tab, window, cx);
        }
        if split.is_some_and(|s| self.live.contains_key(&s)) {
            self.split = split;
        }
        cx.notify();
    }

    /// The window is going away: close its pages.
    fn close_pages(&mut self, cx: &mut App) {
        for live in self.live.values() {
            EngineHub::unregister(cx, live.view.id());
            live.view.close();
        }
        for pane in self.devtools.values() {
            EngineHub::unregister(cx, pane.view.id());
            pane.view.close();
        }
        if let Some(peek) = &self.peek {
            EngineHub::unregister(cx, peek.view.id());
            peek.view.close();
        }
        if let Some(popup) = &self.popup {
            EngineHub::unregister(cx, popup.view.id());
            popup.view.close();
        }
    }
}

/// Whether another window shows `tab` (selected or in its split view).
pub(crate) fn shown_in_other_window(b: &Browser, tab: TabId, cx: &App) -> bool {
    window_showing(b, tab, cx).is_some()
}

/// What other windows do with `tab`'s page.
pub(crate) fn page_elsewhere(b: &Browser, tab: TabId, cx: &App) -> mori_core::windows::OtherWindowPage {
    use mori_core::windows::OtherWindowPage as P;
    if shown_in_other_window(b, tab, cx) {
        P::Shown
    } else if b.kind == WindowKind::Normal && pages_in_other_windows(b, cx).contains(&tab) {
        P::Loaded
    } else {
        P::Nowhere
    }
}

/// Tabs whose pages other windows have loaded.
pub fn pages_in_other_windows(b: &Browser, cx: &App) -> Vec<TabId> {
    if b.kind != WindowKind::Normal {
        return Vec::new();
    }
    let me = b.window_handle.window_id();
    browsers(Some(WindowKind::Normal))
        .iter()
        .filter(|(h, ..)| h.window_id() != me)
        .flat_map(|(_, other, _)| other.read(cx).live.keys().copied().collect::<Vec<_>>())
        .collect()
}

/// Move `tab`'s running page from whichever window has it to `b`.
pub fn take_page_from_other_window(b: &Browser, tab: TabId, cx: &mut App) -> PageElsewhere {
    if b.kind != WindowKind::Normal {
        return PageElsewhere::Nowhere;
    }
    let me = b.window_handle.window_id();
    for (handle, other, _) in browsers(Some(WindowKind::Normal)) {
        if handle.window_id() == me || !other.read(cx).live.contains_key(&tab) {
            continue;
        }
        let detached = update_in(handle, &other, cx, |o, window, cx| {
            let had_focus = o
                .live
                .get(&tab)
                .is_some_and(|l| l.surface.read(cx).focus_handle().is_focused(window));
            let media = o.media.get(&tab).cloned();
            let view = o.detach_page(tab, cx);
            // Keys pressed there next would go to a page that isn't there.
            if view.is_some() && had_focus {
                o.focus_selected_page(window, cx);
            }
            view.map(|v| (v, media))
        });
        // (Without its window, e.g. while it is being updated, the page
        // still moves.)
        let detached = detached.unwrap_or_else(|| {
            other.update(cx, |o, cx| {
                let media = o.media.get(&tab).cloned();
                o.detach_page(tab, cx).map(|v| (v, media))
            })
        });
        return match detached {
            Some((view, media)) => PageElsewhere::Moved(view, media),
            None => PageElsewhere::Busy,
        };
    }
    PageElsewhere::Nowhere
}

/// The other window that shows the page of `tab` (selected or in its
/// split view). A page merely loaded in another window's background
/// doesn't count: selecting it moves it here.
pub fn window_showing(b: &Browser, tab: TabId, cx: &App) -> Option<WindowId> {
    let me = b.window_handle.window_id();
    browsers(Some(WindowKind::Normal))
        .into_iter()
        .filter(|(h, ..)| h.window_id() != me)
        .find(|(_, other, _)| {
            let o = other.read(cx);
            o.live.contains_key(&tab) && (o.selected() == Some(tab) || o.split == Some(tab))
        })
        .map(|(h, ..)| h.window_id())
}

/// Bring window `id` to the front.
pub fn activate(id: WindowId, cx: &mut App) {
    if let Some((handle, ..)) = browser_of(id) {
        handle.update(cx, |_, window, _| window.activate_window()).ok();
    }
}

/// The other normal windows a tab can move to: id and "Space — page".
pub fn other_windows(b: &Browser) -> Vec<(WindowId, String)> {
    let me = b.window_handle.window_id();
    with(|r| {
        r.entries
            .iter()
            .filter(|e| e.kind == WindowKind::Normal && e.handle.window_id() != me)
            .map(|e| (e.handle.window_id(), e.label.clone()))
            .collect()
    })
}

/// Show `tab` in window `target` (a new window with `None`); this window
/// shows another tab if it was showing that one.
pub fn move_tab_to_window(tab: TabId, target: Option<WindowId>, cx: &mut Context<Browser>) {
    let source = cx.entity();
    // Runs after the current event, when no window is being updated.
    cx.defer(move |cx| {
        let source_handle = source.read(cx).window_handle;
        // The tab goes alone: a split pair stays behind.
        source.update(cx, |b, cx| {
            if b.ws.remove_split(tab) {
                b.schedule_save(cx);
            }
        });
        let target = match target {
            Some(id) => browser_of(id).map(|(handle, browser, _)| (handle, browser)),
            None => {
                let mut seed = seed_for(WindowKind::Normal, cx);
                let space = seed.ws.space_of(tab);
                WindowSelection {
                    active_space: space,
                    selected: space.map(|s| vec![(s, tab)]).unwrap_or_default(),
                }
                .apply(&mut seed.ws);
                open(seed, None, cx).map(|b| (b.read(cx).window_handle, b))
            }
        };
        let Some((handle, browser)) = target else { return };
        update_in(handle, &browser, cx, |b, window, cx| {
            b.ws.remove_split(tab);
            b.select_tab(tab, window, cx);
        });
        handle.update(cx, |_, window, _| window.activate_window()).ok();
        update_in(source_handle, &source, cx, |b, window, cx| {
            let shown = b.selected() == Some(tab) || b.split == Some(tab);
            if !shown {
                return;
            }
            match b.ws.neighbor(tab) {
                Some(next) if !shown_in_other_window(b, next, cx) => b.select_tab(next, window, cx),
                // Nothing else to show: the space's new-tab page, without
                // leaving a blank tab behind in every window.
                _ => {
                    b.ws.active_mut().selected = None;
                    b.split = None;
                    b.focus_selected_page(window, cx);
                    b.schedule_save(cx);
                }
            }
        });
    });
}

// ---- Little Mori -----------------------------------------------------------------

/// Keep a Little Mori window to its one page: a tab a page or the user
/// opens there moves to a normal window, and the window closes once its
/// page is closed.
fn tidy_little_window(source: &Entity<Browser>, cx: &mut App) {
    let (handle, own) = {
        let b = source.read(cx);
        let id = b.window_handle.window_id();
        (b.window_handle, with(|r| r.entries.iter().find(|e| e.handle.window_id() == id).and_then(|e| e.little_tab)))
    };
    let Some(own) = own else { return };
    let (extra, own_left): (Vec<TabId>, bool) = {
        let b = source.read(cx);
        let extra = b.ws.tabs.iter().map(|t| t.id).filter(|t| *t != own).collect();
        (extra, b.ws.tab(own).is_some())
    };
    if !own_left {
        handle.update(cx, |_, window, _| window.remove_window()).ok();
        return;
    }
    if extra.is_empty() {
        return;
    }
    // Let the current event finish before windows change.
    let source = source.clone();
    cx.defer(move |cx| {
        let mut moved = Vec::new();
        for tab in extra {
            let taken = source.update(cx, |b, cx| {
                let record = b.ws.tab(tab).cloned();
                let view = b.detach_page(tab, cx);
                if view.is_none() {
                    b.unload(tab, cx);
                }
                b.ws.remove_tab(tab);
                record.map(|r| (r, view))
            });
            if let Some((record, view)) = taken {
                if Browser::is_internal(&record.url) {
                    if let Some(view) = view {
                        view.close();
                    }
                } else {
                    moved.push((record, view));
                }
            }
        }
        update_in(handle, &source, cx, |b, window, cx| {
            if b.selected() != Some(own) {
                b.select_tab(own, window, cx);
            }
            // The command bar that made the tab is gone: keys go to the page.
            b.focus_selected_page(window, cx);
        });
        for (record, view) in moved {
            hand_to_normal_window(record, view, cx);
        }
    });
}

/// The space a Little Mori page would go to now: the current space of the
/// last used normal window, else that of the window that comes back.
fn little_target_space(cx: &App) -> Option<SpaceId> {
    if let Some((_, b, _)) = browsers(Some(WindowKind::Normal)).first() {
        return Some(b.read(cx).ws.active_space);
    }
    with(|r| r.last_closed.clone())
        .or_else(|| WindowSession::load(&mori_core::paths::data_dir()).windows.into_iter().next())
        .and_then(|w| w.selection.active_space)
}

/// Where "Open in …" takes a Little Mori page: the current space of the
/// last used normal window, or just "Mori" when none is open.
pub fn little_target_name(cx: &App) -> String {
    browsers(Some(WindowKind::Normal))
        .first()
        .map(|(_, b, _)| b.read(cx).ws.active().name.clone())
        .unwrap_or_else(|| "Mori".into())
}

/// "Open in Mori": the Little Mori page becomes a tab in the current space
/// of the last used normal window, and the little window closes.
pub fn open_little_in_mori(cx: &mut Context<Browser>) {
    let source = cx.entity();
    cx.defer(move |cx| {
        let handle = source.read(cx).window_handle;
        let Some(tab) = source.read(cx).selected() else { return };
        let taken = source.update(cx, |b, cx| {
            let record = b.ws.tab(tab).cloned();
            let view = b.detach_page(tab, cx);
            record.map(|r| (r, view))
        });
        let Some((record, view)) = taken else { return };
        hand_to_normal_window(record, view, cx);
        handle.update(cx, |_, window, _| window.remove_window()).ok();
    });
}

/// Add a tab (with its running page, if any) to the current space of the
/// last used normal window, select it and bring that window forward.
fn hand_to_normal_window(record: TabRecord, view: Option<WebView>, cx: &mut App) {
    let Some((handle, browser)) = focus_normal_window(cx) else {
        if let Some(view) = view {
            view.close();
        }
        return;
    };
    update_in(handle, &browser, cx, |b, window, cx| {
        // An untouched command bar (a fresh window's) yields to the page.
        let idle_bar = b.launcher.as_ref().is_some_and(|l| {
            let l = l.read(cx);
            l.mode() == crate::ui::launcher::LauncherMode::NewTab && l.query(cx).is_empty()
        });
        if idle_bar {
            b.close_launcher(window, cx);
        }
        let space = b.ws.active_space;
        let id = b.ws.add_tab(record, Some(space), mori_core::workspace::Slot::Loose(0));
        if let Some(view) = view {
            let viewport = window.viewport_size();
            view.resize(f32::from(viewport.width) as i32, f32::from(viewport.height) as i32, window.scale_factor());
            // The little window's surface took the last frame.
            view.invalidate();
            b.adopt_view(id, view, window, cx);
            // It may be playing: the speaker and mini player follow it.
            b.check_media_soon(id, cx);
        }
        b.select_tab(id, window, cx);
    });
}

// ---- Downloads ---------------------------------------------------------------------

/// Hand each download's progress to the window whose page started it (or
/// that heard of it first), falling back to the last used window.
fn route_downloads(updates: Vec<DownloadUpdate>, cx: &mut App) {
    let windows = browsers(None);
    let mut batches: Vec<(WindowId, Vec<DownloadUpdate>)> = Vec::new();
    for update in updates {
        let by_source = update.source.and_then(|view| {
            windows
                .iter()
                .find(|(_, b, _)| {
                    let b = b.read(cx);
                    b.live.values().any(|l| l.view.id() == view)
                        || b.peek.as_ref().is_some_and(|p| p.view.id() == view)
                        || b.popup.as_ref().is_some_and(|p| p.view.id() == view)
                })
                .map(|(h, ..)| h.window_id())
        });
        let known = with(|r| r.downloads.get(&update.id).copied())
            .filter(|id| windows.iter().any(|(h, ..)| h.window_id() == *id));
        let fallback = windows
            .iter()
            .find(|(_, _, k)| *k != WindowKind::Private)
            .or(windows.first())
            .map(|(h, ..)| h.window_id());
        let Some(owner) = known.or(by_source).or(fallback) else { continue };
        with(|r| r.downloads.insert(update.id, owner));
        match batches.iter_mut().find(|(w, _)| *w == owner) {
            Some((_, list)) => list.push(update),
            None => batches.push((owner, vec![update])),
        }
    }
    for (owner, updates) in batches {
        if let Some((handle, browser, _)) = browser_of(owner) {
            update_in(handle, &browser, cx, |b, window, cx| b.on_download_updates(updates, window, cx));
        }
    }
}

// ---- Themes ------------------------------------------------------------------------

/// Keep the theme a window just applied, to draw it with later.
pub fn remember_theme(handle: AnyWindowHandle, cx: &mut App) {
    let Some(tokens) = cx.try_global::<Tokens>().cloned() else { return };
    let theme = CachedTheme {
        tokens,
        kit: KitTheme::global(cx).clone(),
        base: gpui_kit::base::Theme::global(cx),
    };
    let id = handle.window_id();
    let old = with(|r| {
        r.theme_owner = Some(id);
        r.themes.insert(id, theme)
    });
    let Some(old) = old else { return };
    for image in [old.tokens.wash, old.tokens.grain].into_iter().flatten() {
        let current = cx.try_global::<Tokens>().is_some_and(|t| uses_image(t, &image));
        if !current && !theme_image_in_use(&image) {
            cx.drop_image(image, None);
        }
    }
}

/// Put the window's theme into GPUI's globals before it draws.
pub fn use_theme(handle: AnyWindowHandle, cx: &mut App) {
    let id = handle.window_id();
    let theme = with(|r| {
        if r.theme_owner == Some(id) {
            return None;
        }
        let theme = r.themes.get(&id)?;
        r.theme_owner = Some(id);
        Some((theme.tokens.clone(), theme.kit.clone(), theme.base.clone()))
    });
    if let Some((tokens, kit, base)) = theme {
        cx.set_global(tokens);
        *KitTheme::global_mut(cx) = kit;
        cx.set_global(base);
    }
}

fn uses_image(tokens: &Tokens, image: &Arc<RenderImage>) -> bool {
    [&tokens.wash, &tokens.grain]
        .into_iter()
        .flatten()
        .any(|i| Arc::ptr_eq(i, image))
}

/// Whether a window still draws with `image` (a theme's wash or grain).
pub fn theme_image_in_use(image: &Arc<RenderImage>) -> bool {
    with(|r| r.themes.values().any(|t| uses_image(&t.tokens, image)))
}

// ---- Profiles ----------------------------------------------------------------------

/// The browsing context pages of `tab` use in window `b`.
pub fn context_for_tab(b: &Browser, tab: TabId) -> BrowsingContext {
    context_for_space(b, b.ws.space_of(tab).unwrap_or(b.ws.active_space))
}

/// The browsing context pages in `space` of window `b` use.
pub fn context_for_space(b: &Browser, space: SpaceId) -> BrowsingContext {
    match b.kind {
        // A private window never opens without its context.
        WindowKind::Private => b.private_context.clone().unwrap_or_else(|| {
            log::error!("private window without a private context");
            BrowsingContext::private().unwrap_or_else(BrowsingContext::shared)
        }),
        // The profile of the space it was opened for (see `open_little`).
        WindowKind::Little => b.private_context.clone().unwrap_or_else(BrowsingContext::shared),
        WindowKind::Normal => match with(|r| r.profiles.profile_of(space)) {
            None => BrowsingContext::shared(),
            Some(profile) => match BrowsingContext::persistent(&profiles::dir_name(profile)) {
                Some(context) => {
                    b.prepare_profile_context(&context);
                    context
                }
                None => {
                    log::error!("couldn't open profile {profile}; using the default profile");
                    BrowsingContext::shared()
                }
            },
        },
    }
}

pub fn profiles() -> Vec<Profile> {
    with(|r| r.profiles.list().to_vec())
}

/// The named profile `space` uses (`None`: the default profile).
pub fn profile_of(space: SpaceId) -> Option<ProfileId> {
    with(|r| r.profiles.profile_of(space))
}

/// How many spaces use profile `id`.
pub fn spaces_using(id: ProfileId) -> usize {
    with(|r| r.profiles.spaces_of(id).len())
}

pub fn create_profile(name: &str) -> ProfileId {
    let id = with(|r| r.profiles.create(name));
    save_session();
    id
}

pub fn rename_profile(id: ProfileId, name: &str) -> bool {
    let renamed = with(|r| r.profiles.rename(id, name));
    save_session();
    renamed
}

/// Delete a profile: its spaces go back to the default profile (their
/// pages reload there) and its data is removed on the next start.
pub fn delete_profile(id: ProfileId, cx: &mut App) -> Option<DeletedProfile> {
    let deleted = with(|r| r.profiles.delete(id))?;
    save_session();
    refresh_page_contexts(cx);
    Some(deleted)
}

/// Undo [`delete_profile`] (before the next start).
pub fn restore_profile(deleted: DeletedProfile, cx: &mut App) {
    with(|r| r.profiles.restore(deleted));
    save_session();
    refresh_page_contexts(cx);
}

/// Make `space` use `profile` (`None`: the default); its pages reload in
/// the new profile.
pub fn assign_profile(space: SpaceId, profile: Option<ProfileId>, cx: &mut App) {
    if with(|r| r.profiles.assign(space, profile)) {
        save_session();
        refresh_page_contexts(cx);
    }
}

/// After the current event: reload pages that are in the wrong profile.
fn refresh_page_contexts(cx: &mut App) {
    cx.defer(|cx| {
        for (handle, browser, kind) in browsers(Some(WindowKind::Normal)) {
            if kind == WindowKind::Normal {
                update_in(handle, &browser, cx, |b, window, cx| b.fix_page_contexts(window, cx));
            }
        }
    });
}

// ---- Default browser ---------------------------------------------------------------

/// First run: offer once to make Mori the default browser, if it isn't.
fn offer_default_browser(cx: &mut App) {
    let offered = browsers(Some(WindowKind::Normal))
        .first()
        .is_none_or(|(_, b, _)| b.read(cx).settings.default_browser_offered);
    if offered || std::env::var_os("MORI_START_URL").is_some() {
        return;
    }
    let check = cx.background_executor().spawn(async { crate::default_browser::is_default() });
    cx.spawn(async move |cx| {
        let is_default = check.await;
        cx.update(|cx| {
            let Some((handle, browser, _)) = browsers(Some(WindowKind::Normal)).into_iter().next() else { return };
            update_in(handle, &browser, cx, |b, _, cx| {
                b.settings.default_browser_offered = true;
                // The welcome's last step offers it instead.
                if is_default == Some(false) && b.onboarding.is_none() {
                    b.toast(
                        "Make Mori your default browser?",
                        ToastStyle::Info,
                        Some(("Make Default".into(), ToastAction::MakeDefaultBrowser)),
                        cx,
                    );
                }
                b.schedule_save(cx);
            });
        });
    })
    .detach();
}
