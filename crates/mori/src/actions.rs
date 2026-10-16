//! App-wide actions and their default key bindings.
//!
//! `secondary` is Ctrl on Linux/Windows and Cmd on macOS. Like Chrome, Mori
//! lets the page see most shortcuts first: while a page has focus, a
//! shortcut reaches it, and the browser action runs only if the page did not
//! handle the key (no `preventDefault`). Only the [`RESERVED`] set (new or
//! close tab or window, switching tabs and spaces, the address bar, quit,
//! Escape) acts before the page. Unbound keys (Ctrl+A/C/V/X/Z, Tab) always go to the page.

use gpui_kit::{App, KeyBinding, NoAction, actions};

actions!(
    mori,
    [
        NewTab,
        NewWindow,
        NewPrivateWindow,
        CloseWindow,
        OpenLocation,
        CloseTab,
        ReopenClosedTab,
        NextTab,
        PrevTab,
        Reload,
        HardReload,
        Stop,
        Back,
        Forward,
        GoHome,
        Find,
        FindNext,
        FindPrev,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleSidebar,
        ToggleAssistant,
        OpenSettings,
        CopyUrl,
        DuplicateTab,
        TogglePin,
        ToggleBookmark,
        Print,
        ToggleDevTools,
        PeekClipboard,
        BoostSite,
        ZapElement,
        SleepBackgroundTabs,
        ToggleReader,
        CaptureVisibleTab,
        ToggleFullscreen,
        ShowLibrary,
        ShowDownloads,
        NewSpace,
        CloseSplit,
        Dismiss,
        Quit,
        SelectTab1,
        SelectTab2,
        SelectTab3,
        SelectTab4,
        SelectTab5,
        SelectTab6,
        SelectTab7,
        SelectTab8,
        SelectLastTab,
        SelectSpace1,
        SelectSpace2,
        SelectSpace3,
        SelectSpace4,
        SelectSpace5,
        SelectSpace6,
        SelectSpace7,
        SelectSpace8,
        SelectSpace9,
    ]
);

/// Key context of the browser window root.
pub const CONTEXT: &str = "Mori";

/// Key context of a focused web page (see `WebSurface`).
pub const SURFACE_CONTEXT: &str = "WebSurface";

/// Shortcuts the browser handles before the page sees the key, after
/// Chrome's reserved commands: pages can't trap the user in a tab.
pub const RESERVED: &[&str] = &[
    "secondary-t",
    "secondary-n",
    "secondary-shift-n",
    "secondary-w",
    "secondary-shift-w",
    "secondary-shift-t",
    "secondary-l",
    "alt-d",
    "ctrl-tab",
    "ctrl-shift-tab",
    "ctrl-pagedown",
    "ctrl-pageup",
    "secondary-alt-down",
    "secondary-alt-up",
    "secondary-1",
    "secondary-2",
    "secondary-3",
    "secondary-4",
    "secondary-5",
    "secondary-6",
    "secondary-7",
    "secondary-8",
    "secondary-9",
    "alt-1",
    "alt-2",
    "alt-3",
    "alt-4",
    "alt-5",
    "alt-6",
    "alt-7",
    "alt-8",
    "alt-9",
    "secondary-q",
    "escape",
];

/// Keys the window root binds for its own focus and text handling that a
/// focused page must receive instead.
const PAGE_KEYS: &[&str] = &["tab", "shift-tab", "secondary-c"];

/// `(keys, binding)` for every browser shortcut in the [`CONTEXT`].
fn browser_bindings() -> Vec<(&'static str, KeyBinding)> {
    let c = Some(CONTEXT);
    macro_rules! bindings {
        ($($keys:literal => $action:expr),* $(,)?) => {
            vec![$(($keys, KeyBinding::new($keys, $action, c))),*]
        };
    }
    let mut list = bindings![
        "secondary-t" => NewTab,
        "secondary-n" => NewWindow,
        "secondary-shift-n" => NewPrivateWindow,
        "secondary-shift-w" => CloseWindow,
        "secondary-l" => OpenLocation,
        "alt-d" => OpenLocation,
        "secondary-w" => CloseTab,
        "secondary-shift-t" => ReopenClosedTab,
        "ctrl-tab" => NextTab,
        "ctrl-pagedown" => NextTab,
        "secondary-alt-down" => NextTab,
        "ctrl-shift-tab" => PrevTab,
        "ctrl-pageup" => PrevTab,
        "secondary-alt-up" => PrevTab,
        "secondary-r" => Reload,
        "f5" => Reload,
        "secondary-shift-r" => HardReload,
        "shift-f5" => HardReload,
        "secondary-." => Stop,
        "alt-left" => Back,
        "alt-right" => Forward,
        "alt-home" => GoHome,
        "secondary-shift-h" => GoHome,
        "secondary-f" => Find,
        "secondary-g" => FindNext,
        "f3" => FindNext,
        "secondary-shift-g" => FindPrev,
        "shift-f3" => FindPrev,
        "secondary-=" => ZoomIn,
        "secondary-+" => ZoomIn,
        "secondary-shift-=" => ZoomIn,
        "secondary--" => ZoomOut,
        "secondary-0" => ZoomReset,
        "secondary-s" => ToggleSidebar,
        "secondary-shift-k" => ToggleAssistant,
        "secondary-," => OpenSettings,
        "secondary-shift-c" => CopyUrl,
        "secondary-shift-d" => DuplicateTab,
        "secondary-shift-p" => TogglePin,
        "secondary-d" => ToggleBookmark,
        "secondary-p" => Print,
        "secondary-shift-i" => ToggleDevTools,
        "f12" => ToggleDevTools,
        "secondary-shift-o" => PeekClipboard,
        "secondary-shift-b" => BoostSite,
        "secondary-alt-s" => SleepBackgroundTabs,
        "secondary-alt-r" => ToggleReader,
        "f11" => ToggleFullscreen,
        "secondary-y" => ShowLibrary,
        "secondary-h" => ShowLibrary,
        "secondary-shift-j" => ShowDownloads,
        "secondary-shift-s" => CloseSplit,
        "escape" => Dismiss,
        "secondary-q" => Quit,
        "secondary-1" => SelectTab1,
        "secondary-2" => SelectTab2,
        "secondary-3" => SelectTab3,
        "secondary-4" => SelectTab4,
        "secondary-5" => SelectTab5,
        "secondary-6" => SelectTab6,
        "secondary-7" => SelectTab7,
        "secondary-8" => SelectTab8,
        "secondary-9" => SelectLastTab,
        "alt-1" => SelectSpace1,
        "alt-2" => SelectSpace2,
        "alt-3" => SelectSpace3,
        "alt-4" => SelectSpace4,
        "alt-5" => SelectSpace5,
        "alt-6" => SelectSpace6,
        "alt-7" => SelectSpace7,
        "alt-8" => SelectSpace8,
        "alt-9" => SelectSpace9,
    ];
    // Cmd+[ / Cmd+] are the macOS history shortcuts; on Linux Ctrl+[ is
    // Escape in terminals and outdent in editors, and Chrome leaves it alone.
    if cfg!(target_os = "macos") {
        list.extend(bindings!["secondary-[" => Back, "secondary-]" => Forward]);
    }
    list
}

pub fn bind_keys(cx: &mut App) {
    let page = Some(SURFACE_CONTEXT);
    let mut bindings = Vec::new();
    for (keys, binding) in browser_bindings() {
        bindings.push(binding);
        // Hide non-reserved shortcuts from the focused page's key context so
        // the key goes to the page first; `WebSurface` runs the action if
        // the page leaves the key unhandled.
        if !RESERVED.contains(&keys) {
            bindings.push(KeyBinding::new(keys, NoAction, page));
        }
    }
    for keys in PAGE_KEYS {
        bindings.push(KeyBinding::new(keys, NoAction, page));
    }
    cx.bind_keys(bindings);
}

/// Human-readable shortcut for menus and the launcher, e.g. "Ctrl+Shift+T".
pub fn shortcut_label(keys: &str) -> String {
    let secondary = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };
    let mut parts = Vec::new();
    let mut rest = keys;
    // Modifiers are prefixes; whatever remains is the key (which may be "-").
    while let Some((modifier, tail)) = rest.split_once('-') {
        let label = match modifier {
            "secondary" => secondary,
            "ctrl" => "Ctrl",
            "shift" => "Shift",
            "alt" => "Alt",
            "cmd" | "super" => "Super",
            _ => break,
        };
        if tail.is_empty() {
            break;
        }
        parts.push(label.to_string());
        rest = tail;
    }
    let key = match rest {
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => {
            let mut c = k.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        }
    };
    parts.push(key);
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use super::{RESERVED, browser_bindings, shortcut_label};

    #[test]
    fn reserved_shortcuts_are_bound_and_page_shortcuts_are_not_reserved() {
        let keys: Vec<&str> = browser_bindings().into_iter().map(|(k, _)| k).collect();
        for reserved in RESERVED {
            assert!(keys.contains(reserved), "{reserved} is reserved but unbound");
        }
        // Shortcuts web apps rely on reach the page first.
        for page_first in ["secondary-s", "secondary-f", "secondary-p", "secondary-d", "secondary-,"] {
            assert!(keys.contains(&page_first));
            assert!(!RESERVED.contains(&page_first));
        }
        #[cfg(not(target_os = "macos"))]
        assert!(!keys.contains(&"secondary-["));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn labels() {
        assert_eq!(shortcut_label("secondary-shift-t"), "Ctrl+Shift+T");
        assert_eq!(shortcut_label("secondary--"), "Ctrl+-");
        assert_eq!(shortcut_label("f12"), "F12");
        assert_eq!(shortcut_label("alt-left"), "Alt+Left");
    }
}
