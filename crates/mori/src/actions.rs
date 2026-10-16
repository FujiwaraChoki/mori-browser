//! App-wide actions and their default key bindings.
//!
//! `secondary` is Ctrl on Linux/Windows and Cmd on macOS. Bindings resolve
//! before keys reach the page, so anything bound here is a browser shortcut;
//! unbound keys (including Ctrl+A/C/V/X/Z) go to the web view.

use gpui_kit::{App, KeyBinding, actions};

actions!(
    mori,
    [
        NewTab,
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

pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-t", NewTab, c),
        KeyBinding::new("secondary-l", OpenLocation, c),
        KeyBinding::new("alt-d", OpenLocation, c),
        KeyBinding::new("secondary-w", CloseTab, c),
        KeyBinding::new("secondary-shift-t", ReopenClosedTab, c),
        KeyBinding::new("ctrl-tab", NextTab, c),
        KeyBinding::new("ctrl-pagedown", NextTab, c),
        KeyBinding::new("secondary-alt-down", NextTab, c),
        KeyBinding::new("ctrl-shift-tab", PrevTab, c),
        KeyBinding::new("ctrl-pageup", PrevTab, c),
        KeyBinding::new("secondary-alt-up", PrevTab, c),
        KeyBinding::new("secondary-r", Reload, c),
        KeyBinding::new("f5", Reload, c),
        KeyBinding::new("secondary-shift-r", HardReload, c),
        KeyBinding::new("shift-f5", HardReload, c),
        KeyBinding::new("secondary-.", Stop, c),
        KeyBinding::new("alt-left", Back, c),
        KeyBinding::new("secondary-[", Back, c),
        KeyBinding::new("alt-right", Forward, c),
        KeyBinding::new("secondary-]", Forward, c),
        KeyBinding::new("alt-home", GoHome, c),
        KeyBinding::new("secondary-shift-h", GoHome, c),
        KeyBinding::new("secondary-f", Find, c),
        KeyBinding::new("secondary-g", FindNext, c),
        KeyBinding::new("f3", FindNext, c),
        KeyBinding::new("secondary-shift-g", FindPrev, c),
        KeyBinding::new("shift-f3", FindPrev, c),
        KeyBinding::new("secondary-=", ZoomIn, c),
        KeyBinding::new("secondary-+", ZoomIn, c),
        KeyBinding::new("secondary-shift-=", ZoomIn, c),
        KeyBinding::new("secondary--", ZoomOut, c),
        KeyBinding::new("secondary-0", ZoomReset, c),
        KeyBinding::new("secondary-s", ToggleSidebar, c),
        KeyBinding::new("secondary-shift-k", ToggleAssistant, c),
        KeyBinding::new("secondary-,", OpenSettings, c),
        KeyBinding::new("secondary-shift-c", CopyUrl, c),
        KeyBinding::new("secondary-shift-d", DuplicateTab, c),
        KeyBinding::new("secondary-shift-p", TogglePin, c),
        KeyBinding::new("secondary-d", ToggleBookmark, c),
        KeyBinding::new("secondary-p", Print, c),
        KeyBinding::new("secondary-shift-i", ToggleDevTools, c),
        KeyBinding::new("f12", ToggleDevTools, c),
        KeyBinding::new("secondary-shift-o", PeekClipboard, c),
        KeyBinding::new("secondary-shift-b", BoostSite, c),
        KeyBinding::new("secondary-alt-s", SleepBackgroundTabs, c),
        KeyBinding::new("secondary-alt-r", ToggleReader, c),
        KeyBinding::new("f11", ToggleFullscreen, c),
        KeyBinding::new("secondary-y", ShowLibrary, c),
        KeyBinding::new("secondary-h", ShowLibrary, c),
        KeyBinding::new("secondary-shift-j", ShowDownloads, c),
        KeyBinding::new("secondary-shift-s", CloseSplit, c),
        KeyBinding::new("escape", Dismiss, c),
        KeyBinding::new("secondary-q", Quit, c),
        KeyBinding::new("secondary-1", SelectTab1, c),
        KeyBinding::new("secondary-2", SelectTab2, c),
        KeyBinding::new("secondary-3", SelectTab3, c),
        KeyBinding::new("secondary-4", SelectTab4, c),
        KeyBinding::new("secondary-5", SelectTab5, c),
        KeyBinding::new("secondary-6", SelectTab6, c),
        KeyBinding::new("secondary-7", SelectTab7, c),
        KeyBinding::new("secondary-8", SelectTab8, c),
        KeyBinding::new("secondary-9", SelectLastTab, c),
        KeyBinding::new("alt-1", SelectSpace1, c),
        KeyBinding::new("alt-2", SelectSpace2, c),
        KeyBinding::new("alt-3", SelectSpace3, c),
        KeyBinding::new("alt-4", SelectSpace4, c),
        KeyBinding::new("alt-5", SelectSpace5, c),
        KeyBinding::new("alt-6", SelectSpace6, c),
        KeyBinding::new("alt-7", SelectSpace7, c),
        KeyBinding::new("alt-8", SelectSpace8, c),
        KeyBinding::new("alt-9", SelectSpace9, c),
    ]);
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
    use super::shortcut_label;

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn labels() {
        assert_eq!(shortcut_label("secondary-shift-t"), "Ctrl+Shift+T");
        assert_eq!(shortcut_label("secondary--"), "Ctrl+-");
        assert_eq!(shortcut_label("f12"), "F12");
        assert_eq!(shortcut_label("alt-left"), "Alt+Left");
    }
}
