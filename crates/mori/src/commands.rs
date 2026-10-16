//! Named commands, shared by the launcher and menus.

use gpui_kit::*;
use mori_core::workspace::SpaceId;

use crate::{
    browser::{Browser, Panel, RenameTarget, ToastStyle},
    ui::launcher::LauncherMode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    NewTab,
    NewSpace,
    NewFolderWithSelected,
    ReaderView,
    CaptureVisibleTab,
    Capture,
    CaptureFullPage,
    KeyboardShortcuts,
    ToggleCompactMode,
    ImportBrowserData,
    BoostSite,
    ZapElement,
    PeekClipboard,
    SleepBackgroundTabs,
    ReopenClosedTab,
    FindInPage,
    ToggleSidebar,
    Settings,
    RenameSpace,
    ClearDownloads,
    CopyUrl,
    TogglePin,
    RenameTab,
    DuplicateTab,
    ToggleBookmark,
    OpenAssistant,
    DevTools,
    Library,
    Downloads,
    Print,
    MoveTabToSpace(SpaceId),
    SwitchToSpace(SpaceId),
}

pub struct CommandInfo {
    pub command: Command,
    pub title: String,
    pub keywords: &'static str,
    pub icon: &'static str,
    pub shortcut: Option<&'static str>,
}

/// Commands available right now, in launcher order.
pub fn available(browser: &Browser) -> Vec<CommandInfo> {
    let mut out = Vec::new();
    let mut add = |command, title: &str, keywords, icon, shortcut| {
        out.push(CommandInfo {
            command,
            title: title.to_string(),
            keywords,
            icon,
            shortcut,
        })
    };
    use Command::*;
    add(NewTab, "New Tab", "open blank", "plus", Some("secondary-t"));
    add(ReaderView, "Reader View", "read article distraction", "book-open-text", Some("secondary-alt-r"));
    add(CaptureVisibleTab, "Capture Visible Tab", "screenshot image", "camera", None);
    add(Capture, "Capture…", "screenshot region area image crop", "scan", Some("secondary-shift-x"));
    add(CaptureFullPage, "Capture Full Page", "screenshot whole long image", "scan-line", None);
    add(BoostSite, "Boost This Site…", "css js customize", "zap", Some("secondary-shift-b"));
    add(ZapElement, "Zap an Element", "hide remove element boost", "eraser", None);
    add(PeekClipboard, "Peek a Link", "preview clipboard", "eye", Some("secondary-shift-o"));
    add(SleepBackgroundTabs, "Sleep Background Tabs", "memory unload", "moon", Some("secondary-alt-s"));
    add(ReopenClosedTab, "Reopen Closed Tab", "undo restore", "rotate-ccw", Some("secondary-shift-t"));
    add(FindInPage, "Find in Page", "search text", "search", Some("secondary-f"));
    add(ToggleSidebar, "Toggle Sidebar", "hide show", "panel-left", Some("secondary-s"));
    add(
        ToggleCompactMode,
        if browser.settings.compact_mode { "Exit Compact Mode" } else { "Compact Mode" },
        "hide sidebar focus zen fullscreen",
        "panel-left-close",
        Some("secondary-alt-c"),
    );
    add(KeyboardShortcuts, "Keyboard Shortcuts", "keys cheatsheet hotkeys help", "keyboard", Some("secondary-/"));
    add(ImportBrowserData, "Import from Another Browser…", "chrome firefox zen arc bookmarks history migrate", "import", None);
    add(Settings, "Settings", "preferences options", "settings", Some("secondary-,"));
    add(Library, "History & Bookmarks", "library archive", "library", Some("secondary-y"));
    add(Downloads, "Downloads", "files", "download", Some("secondary-shift-j"));
    add(NewSpace, "New Space", "context create", "layers", None);
    add(RenameSpace, "Rename Space", "context", "pencil", None);
    add(ClearDownloads, "Clear Finished Downloads", "", "trash", None);
    add(DevTools, "Developer Tools", "inspect console", "code", Some("secondary-shift-i"));
    add(Print, "Print…", "pdf", "printer", Some("secondary-p"));
    if browser.multi_select.len() >= 2 {
        let n = browser.multi_select.len();
        add(NewFolderWithSelected, &format!("New Folder with {n} Selected Tabs"), "group", "folder-plus", None);
    }
    if let Some(id) = browser.selected() {
        let pinned = browser.ws.is_pinned(id);
        let url = browser.ws.tab(id).map(|t| t.url.clone()).unwrap_or_default();
        let bookmarked = browser.bookmarks.contains(&url);
        add(CopyUrl, "Copy URL", "link clipboard", "link", Some("secondary-shift-c"));
        add(TogglePin, if pinned { "Unpin Tab" } else { "Pin Tab" }, "favorite", "pin", Some("secondary-shift-p"));
        add(RenameTab, "Rename Tab", "title", "pencil", None);
        add(DuplicateTab, "Duplicate Tab", "copy clone", "copy", Some("secondary-shift-d"));
        add(
            ToggleBookmark,
            if bookmarked { "Remove Bookmark" } else { "Bookmark Page" },
            "star favorite",
            "star",
            Some("secondary-d"),
        );
        for space in browser.ws.spaces.iter().filter(|s| s.id != browser.ws.active_space) {
            add(MoveTabToSpace(space.id), &format!("Move Tab to {}", space.name), "space context", "arrow-right-left", None);
        }
    }
    if browser.settings.ai_integration_enabled {
        add(OpenAssistant, "Open Assistant", "ai chat codex agent", "sparkles", Some("secondary-shift-k"));
    }
    for space in browser.ws.spaces.iter().filter(|s| s.id != browser.ws.active_space) {
        add(SwitchToSpace(space.id), &format!("Switch to {}", space.name), "space context", "layers", None);
    }
    out
}

pub fn run(b: &mut Browser, command: Command, window: &mut Window, cx: &mut Context<Browser>) {
    use Command::*;
    match command {
        // Like Ctrl+T: the tab is created when the launcher is submitted, so
        // a "Homepage" new-tab setting doesn't leave an extra tab behind.
        NewTab => b.open_launcher(LauncherMode::NewTab, window, cx),
        NewSpace => b.create_space(window, cx),
        NewFolderWithSelected => {
            let tabs = b.multi_select.clone();
            b.new_folder(tabs, window, cx);
        }
        ReaderView => b.toggle_reader(cx),
        CaptureVisibleTab => crate::screenshot::capture_visible_tab(b, cx),
        Capture => b.start_capture(window, cx),
        CaptureFullPage => match b.active_tab() {
            Some(tab) => {
                let destination = b.capture_destination;
                b.capture_full_page(tab, destination, cx);
            }
            None => b.toast("Open a web page to capture it", ToastStyle::Info, None, cx),
        },
        KeyboardShortcuts => b.toggle_cheatsheet(window, cx),
        ToggleCompactMode => b.toggle_compact_mode(cx),
        ImportBrowserData => b.show_settings(crate::ui::settings::SettingsCategory::Import, window, cx),
        BoostSite => b.open_boost_editor(window, cx),
        ZapElement => b.start_zap(cx),
        PeekClipboard => peek_clipboard(b, window, cx),
        SleepBackgroundTabs => b.sleep_background_tabs(cx),
        ReopenClosedTab => b.reopen_closed_tab(window, cx),
        FindInPage => b.open_find(window, cx),
        ToggleSidebar => b.toggle_sidebar(cx),
        Settings => b.set_settings_open(true, window, cx),
        RenameSpace => {
            let id = b.ws.active_space;
            b.begin_rename(RenameTarget::Space(id), window, cx);
        }
        ClearDownloads => b.clear_finished_downloads(cx),
        CopyUrl => b.copy_url(cx),
        TogglePin => {
            if let Some(id) = b.selected() {
                b.toggle_pin(id, cx);
            }
        }
        RenameTab => {
            if let Some(id) = b.selected() {
                b.begin_rename(RenameTarget::Tab(id), window, cx);
            }
        }
        DuplicateTab => {
            if let Some(id) = b.selected() {
                b.duplicate_tab(id, window, cx);
            }
        }
        ToggleBookmark => b.toggle_bookmark(cx),
        OpenAssistant => b.open_assistant(None, window, cx),
        DevTools => b.toggle_devtools(window, cx),
        Library => {
            b.panel = Some(Panel::Library);
            cx.notify();
        }
        Downloads => {
            b.panel = Some(Panel::Downloads);
            cx.notify();
        }
        Print => b.print_selected(cx),
        MoveTabToSpace(space) => {
            if let Some(id) = b.selected() {
                b.move_tab_to_space(id, space, window, cx);
                let name = b.ws.space(space).map(|s| s.name.clone()).unwrap_or_default();
                b.toast(format!("Moved to {name}"), ToastStyle::Success, None, cx);
            }
        }
        SwitchToSpace(space) => b.switch_space(space, window, cx),
    }
}

/// Peek the URL on the clipboard, or the current page if there is none.
pub fn peek_clipboard(b: &mut Browser, window: &mut Window, cx: &mut Context<Browser>) {
    let text = cx
        .read_from_clipboard()
        .and_then(|c| c.text())
        .map(|t| t.trim().to_string())
        .filter(|t| mori_core::omnibox::looks_like_address(t));
    let url = match text {
        Some(t) => match mori_core::omnibox::resolve(&t, &b.settings) {
            mori_core::omnibox::Resolved::Url(u) | mori_core::omnibox::Resolved::Search(u) => u,
        },
        None => match b.selected().and_then(|id| b.ws.tab(id)) {
            Some(t) if !Browser::is_internal(&t.url) => t.url.clone(),
            _ => {
                b.toast("Copy a link to peek it", ToastStyle::Info, None, cx);
                return;
            }
        },
    };
    b.peek_url(&url, window, cx);
}
