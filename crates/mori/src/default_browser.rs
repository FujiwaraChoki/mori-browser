//! Mori as the desktop's default web browser (freedesktop `xdg-settings`).
//!
//! Registering needs a desktop entry named `mori.desktop` that handles web
//! links. A packaged or installed Mori has one (`packaging/linux`,
//! `scripts/install-linux.sh`); a Mori run straight from a build writes one
//! for the current user that starts this executable.

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    process::Command,
};

use gpui_kit::*;

use crate::browser::{Browser, ToastStyle};

pub const DESKTOP_FILE: &str = "mori.desktop";

/// The packaged desktop entry; `Exec=mori` is replaced for a user entry.
const DESKTOP_ENTRY: &str = include_str!("../../../packaging/linux/mori.desktop");

/// MIME types Mori takes over as the default browser.
const WEB_TYPES: [&str; 4] = [
    "x-scheme-handler/http",
    "x-scheme-handler/https",
    "text/html",
    "application/xhtml+xml",
];

thread_local! {
    /// Last known answer of [`is_default`] (`None`: unknown or unchecked).
    static STATUS: Cell<Option<bool>> = const { Cell::new(None) };
    static CHECKED: Cell<bool> = const { Cell::new(false) };
}

/// Whether Mori is the default browser; `None` if that can't be told
/// (no `xdg-settings`).
pub fn is_default() -> Option<bool> {
    let out = Command::new("xdg-settings")
        .args(["check", "default-web-browser", DESKTOP_FILE])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    match String::from_utf8_lossy(&out.stdout).trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// The last known status for the settings page; the first call starts a
/// check and redraws the windows when it is done.
pub fn status(cx: &mut App) -> Option<bool> {
    if !CHECKED.with(|c| c.replace(true)) {
        refresh(cx);
    }
    STATUS.with(Cell::get)
}

fn refresh(cx: &mut App) {
    let check = cx.background_executor().spawn(async { is_default() });
    cx.spawn(async move |cx| {
        let status = check.await;
        STATUS.with(|s| s.set(status));
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}

/// Make Mori the default browser, then report how it went in a toast.
pub fn make_default(cx: &mut Context<Browser>) {
    let exe = std::env::current_exe().ok();
    let work = cx.background_executor().spawn(async move { register(exe.as_deref()) });
    cx.spawn(async move |this, cx| {
        let result = work.await;
        let status = is_default();
        STATUS.with(|s| s.set(status));
        this.update(cx, |b, cx| {
            match result {
                Ok(()) if status != Some(false) => {
                    b.toast("Mori is now your default browser", ToastStyle::Success, None, cx)
                }
                Ok(()) => b.toast("Your desktop kept another default browser", ToastStyle::Warning, None, cx),
                Err(e) => {
                    log::warn!("setting the default browser: {e:#}");
                    b.toast("Couldn't make Mori the default browser", ToastStyle::Error, None, cx)
                }
            }
            cx.refresh_windows();
        })
        .ok();
    })
    .detach();
}

/// Register `mori.desktop` for web links, writing a user desktop entry for
/// `exe` first if no installed one exists.
fn register(exe: Option<&Path>) -> anyhow::Result<()> {
    if find_desktop_entry().is_none() {
        let exe = exe.ok_or_else(|| anyhow::anyhow!("no executable path"))?;
        write_user_entry(exe)?;
    }
    let settings = Command::new("xdg-settings")
        .args(["set", "default-web-browser", DESKTOP_FILE])
        .status();
    // `xdg-mime` covers HTML files too, and desktops `xdg-settings` doesn't
    // know.
    let mime = Command::new("xdg-mime").arg("default").arg(DESKTOP_FILE).args(WEB_TYPES).status();
    match (settings, mime) {
        (Ok(s), _) if s.success() => Ok(()),
        (_, Ok(m)) if m.success() => Ok(()),
        (Err(e), Err(_)) => Err(anyhow::anyhow!("xdg-settings and xdg-mime unavailable: {e}")),
        _ => Err(anyhow::anyhow!("xdg-settings and xdg-mime failed")),
    }
}

/// The XDG data directories, user first.
fn data_dirs() -> Vec<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let home = env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")));
    let system = env("XDG_DATA_DIRS")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["/usr/local/share".into(), "/usr/share".into()]);
    home.into_iter().chain(system).collect()
}

fn find_desktop_entry() -> Option<PathBuf> {
    data_dirs()
        .into_iter()
        .map(|d| d.join("applications").join(DESKTOP_FILE))
        .find(|p| p.is_file())
}

fn write_user_entry(exe: &Path) -> anyhow::Result<()> {
    let dir = data_dirs()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no XDG data directory"))?
        .join("applications");
    std::fs::create_dir_all(&dir)?;
    let entry = user_entry(&exec_arg(&exe.to_string_lossy()));
    mori_core::paths::write_bytes_atomic(&dir.join(DESKTOP_FILE), entry.as_bytes())?;
    // Let desktops pick it up; not every system has the tool.
    let _ = Command::new("update-desktop-database").arg(&dir).status();
    Ok(())
}

/// The packaged entry with its `Exec` lines starting `program`.
fn user_entry(program: &str) -> String {
    DESKTOP_ENTRY
        .lines()
        .map(|line| match line.strip_prefix("Exec=mori") {
            Some(rest) => format!("Exec={program}{rest}"),
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// `path` as one argument of a desktop entry's `Exec` key: quoted when it
/// holds anything beyond plain path characters, with `"`, `` ` ``, `$` and
/// `\` escaped; `%` is doubled either way.
fn exec_arg(path: &str) -> String {
    let path = path.replace('%', "%%");
    let plain = path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+%".contains(c));
    if plain {
        return path;
    }
    let mut out = String::from("\"");
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::{WEB_TYPES, exec_arg, user_entry};

    #[test]
    fn exec_arguments_are_quoted_when_needed() {
        assert_eq!(exec_arg("/home/a/mori/target/debug/mori"), "/home/a/mori/target/debug/mori");
        assert_eq!(exec_arg("/opt/My Apps/mori"), "\"/opt/My Apps/mori\"");
        assert_eq!(exec_arg("/tmp/$x\"y"), "\"/tmp/\\$x\\\"y\"");
        assert_eq!(exec_arg("/tmp/100%/mori"), "/tmp/100%%/mori");
    }

    #[test]
    fn the_user_entry_starts_this_build_with_links() {
        let entry = user_entry("/opt/mori/mori");
        assert!(entry.contains("Exec=/opt/mori/mori %U\n"));
        assert!(entry.contains("Exec=/opt/mori/mori --new-window\n"));
        assert!(entry.contains("Exec=/opt/mori/mori --private-window\n"));
        assert!(!entry.contains("Exec=mori"));
        for mime in WEB_TYPES {
            assert!(entry.contains(mime), "{mime} missing from MimeType");
        }
    }
}
