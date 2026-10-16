//! The browser window's model: workspace, stores, live tabs, and commands.
//!
//! `Browser` is the root GPUI entity of a window. It owns the persisted
//! workspace (tabs, spaces, folders) and the stores, keeps a [`LiveTab`] for
//! every tab whose page is loaded, translates Chromium events into model
//! changes, and implements every user command. Rendering lives in `ui/`.

use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use gpui_kit::*;
use mori_core::{
    archive::ArchiveStore,
    bookmarks::BookmarkStore,
    boosts::{self, BoostStore},
    downloads::DownloadStore,
    history::HistoryStore,
    maintenance::{self, TabActivity},
    navigation,
    omnibox::{self, Resolved},
    persist::Saver,
    prompts::PromptQueue,
    routes::{Route, RouteStore},
    settings::Settings,
    site_permissions::{Decision, SitePermissionStore},
    util::host_of,
    workspace::{
        ClosedTab, DeletedSpace, FolderId, Slot, SpaceId, TabId, TabRecord, Workspace,
    },
    zoom::ZoomStore,
};
use mori_engine::{
    AuthRequest, CertificateError, ContextMenuInfo, DialogKind, DownloadId, JsDialog,
    PermissionRequest, ViewEvent, WebView, keys::KeyInput,
};

use crate::{
    engine_hub::EngineHub,
    favicons::Favicons,
    ui::{
        find_bar::{FindBar, FindBarEvent},
        launcher::{Launcher, LauncherEvent, LauncherMode},
    },
    web_surface::WebSurface,
};

mod devtools;
mod downloads;
mod page_tools;
mod popups;

pub use devtools::DevToolsPane;
pub use popups::PopupWindow;

type InputState = gpui_kit::component::input::InputState;

/// How far from the window edge the pointer reveals a hidden sidebar.
const SIDEBAR_REVEAL_EDGE: f32 = 6.;
/// How far past the floating sidebar the pointer may stray before it hides.
const SIDEBAR_HIDE_MARGIN: f32 = 24.;
/// How long the pointer must stay away before the floating sidebar hides.
const SIDEBAR_HIDE_DELAY: Duration = Duration::from_millis(300);

/// How long a page may take to answer a close request (run `beforeunload`)
/// before another close request closes it without asking.
const CLOSE_ANSWER_TIME: Duration = Duration::from_secs(1);

/// Runtime state of a tab whose page is loaded in Chromium.
pub struct LiveTab {
    pub surface: Entity<WebSurface>,
    pub view: WebView,
    pub loading: bool,
    pub progress: f64,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub error: Option<PageError>,
    pub find_count: i32,
    pub find_active: i32,
    pub reader: bool,
    pub fullscreen: bool,
    pub status: Option<String>,
    pub muted: bool,
    /// The tab whose page opened this one (target=_blank, window.open).
    pub opener: Option<TabId>,
    /// Opened by a page and has not shown a page yet: if its first
    /// navigation turns into a download, the tab is just dropped.
    pub blank_popup: bool,
    /// The user asked to close the tab and the page's `beforeunload` is
    /// running (since when); the tab goes once the page agrees.
    pub close_requested: Option<Instant>,
    _surface_events: Subscription,
}

/// The page a prompt belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageOwner {
    Tab(TabId),
    Peek,
    /// The floating `window.open` popup.
    Popup,
}

/// A JavaScript dialog waiting for the user.
pub struct PendingDialog {
    pub dialog: JsDialog,
    /// The text field of a `prompt()`.
    pub input: Option<Entity<InputState>>,
    _subscription: Option<Subscription>,
}

/// An HTTP authentication request waiting for credentials.
pub struct PendingAuth {
    pub request: AuthRequest,
    pub user: Entity<InputState>,
    pub password: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug)]
pub struct PageError {
    pub code: i32,
    pub text: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastStyle {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub enum ToastAction {
    UndoClose(Box<ClosedTab>),
    /// Undo a bulk close ("Close Other Tabs"), in closing order.
    UndoCloseMany(Vec<ClosedTab>),
    /// Undo "Archive Tab": reopen it and drop the archive entry it made.
    UndoArchive {
        closed: Box<ClosedTab>,
        entry: uuid::Uuid,
    },
    UndoDeleteSpace(Box<(DeletedSpace, Vec<Route>)>),
    UndoClearHistory(Box<HistoryStore>),
    Unpin(TabId),
    OpenDownloads,
    /// Show a file in the file manager.
    Reveal(PathBuf),
    ResetZoom,
    /// Print a tab (a page asked to with `window.print()`).
    PrintTab(TabId),
    /// Print the Peek page.
    PrintPeek,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub text: SharedString,
    pub style: ToastStyle,
    pub action: Option<(SharedString, ToastAction)>,
    /// Playing its exit animation; removed shortly after.
    pub leaving: bool,
    /// Bumped when a keyed toast is updated in place, so the timer of its
    /// earlier text doesn't dismiss the new one.
    serial: u64,
}

/// How long a toast takes to fade out before it is removed.
pub const TOAST_EXIT: Duration = Duration::from_millis(160);

/// A right-click menu currently on screen.
#[derive(Clone, Debug)]
pub enum MenuKind {
    Web { tab: TabId, info: Box<ContextMenuInfo> },
    Tab(TabId),
    Folder(FolderId),
    Space(SpaceId),
    Sidebar,
    NewMenu,
    /// Connection, permissions and zoom for the selected page (URL pill).
    SiteInfo,
}

#[derive(Clone, Debug)]
pub struct OpenMenu {
    pub kind: MenuKind,
    /// Window coordinates.
    pub position: Point<Pixels>,
}

/// What is being renamed inline in the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameTarget {
    Tab(TabId),
    Folder(FolderId),
    Space(SpaceId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Downloads,
    Library,
}

pub struct Peek {
    pub surface: Entity<WebSurface>,
    pub view: WebView,
    pub title: String,
    pub url: String,
    _surface_events: Subscription,
    pub loading: bool,
}

pub struct Browser {
    pub ws: Workspace,
    pub settings: Settings,
    pub history: HistoryStore,
    pub bookmarks: BookmarkStore,
    pub archive: ArchiveStore,
    pub routes: RouteStore,
    pub boosts: BoostStore,
    pub site_permissions: SitePermissionStore,
    pub favicons: Favicons,

    pub live: HashMap<TabId, LiveTab>,
    pub downloads: DownloadStore,
    pub downloads_dir: PathBuf,
    /// Which tab started each download, so its page isn't slept mid-way.
    download_tabs: HashMap<DownloadId, TabId>,
    pub zoom: ZoomStore,
    /// DevTools docked next to the page of these tabs.
    pub devtools: HashMap<TabId, DevToolsPane>,
    /// A `window.open` popup that asked for a size, floating over its opener.
    pub popup: Option<PopupWindow>,
    pub toasts: Vec<Toast>,
    next_toast: u64,
    /// Toasts that update in place (zoom level, download progress).
    keyed_toasts: HashMap<&'static str, u64>,
    /// The window is being presented (not minimized, not on another
    /// workspace); pages are hidden from Chromium while it is not.
    window_visible: bool,

    pub launcher: Option<Entity<Launcher>>,
    launcher_events: Option<Subscription>,
    pub find_bar: Option<Entity<FindBar>>,
    pub menu: Option<OpenMenu>,
    pub panel: Option<Panel>,
    pub settings_open: bool,
    pub peek: Option<Peek>,
    pub dialogs: PromptQueue<PageOwner, PendingDialog>,
    /// Focus target of an alert/confirm dialog, so Enter answers it.
    pub dialog_focus: FocusHandle,
    pub auth_prompts: PromptQueue<PageOwner, PendingAuth>,
    pub permissions: PromptQueue<PageOwner, PermissionRequest>,
    pub cert_errors: PromptQueue<PageOwner, CertificateError>,
    /// Hosts the user let through despite a bad certificate, this session.
    pub cert_exceptions: mori_core::security::CertificateExceptions,
    /// When the user last Alt+clicked in the window; a page's Peek request
    /// (PEEK_LINK_SCRIPT) is only honoured right after one.
    pub alt_click_at: Option<Instant>,
    pub rename: Option<(RenameTarget, Entity<gpui_kit::component::input::InputState>)>,
    rename_events: Option<Subscription>,
    pub boost_editor: Option<Entity<crate::ui::boost_editor::BoostEditor>>,
    boost_events: Option<Subscription>,
    pub zapping: Option<TabId>,
    pub sidebar_peek: bool,
    /// Pending auto-hide of the floating sidebar; dropping it cancels.
    sidebar_hide: Option<Task<()>>,
    pub sidebar_drag: Option<f32>,
    pub multi_select: Vec<TabId>,
    /// The assistant panel, kept alive while hidden so its conversation and
    /// Codex server survive toggling.
    pub assistant: Option<Entity<crate::ui::assistant::AssistantPanel>>,
    pub assistant_visible: bool,
    /// The right pane of the split view on screen. The selected tab is
    /// always the left pane; the pair itself lives in the workspace.
    pub split: Option<TabId>,
    /// Whether the right split pane (rather than the selected tab) has the
    /// user's attention: toolbar, omnibox and page shortcuts act on it.
    pub split_focused: bool,
    /// Divider drag in progress: the left pane's share of the web area.
    pub split_drag: Option<f32>,
    /// The last background tab opened from a page (opener, child), so the
    /// next one lands after it.
    child_anchor: Option<(TabId, TabId)>,
    /// Typed addresses Mori upgraded to https, retried over http if the
    /// https load fails.
    typed_https: HashMap<TabId, String>,
    /// Scroll offsets to restore once a woken or restored page loads.
    pending_scroll: HashMap<TabId, f64>,
    pub media: HashMap<TabId, crate::ui::media::TabMedia>,
    /// Tabs with a media check scheduled (see `check_media_soon`).
    pub media_checks: std::collections::HashSet<TabId>,
    /// Clipboard bridge: last copy sequence seen per view, and the text last
    /// pushed into Chromium's clipboard.
    clip_seq: HashMap<mori_engine::ViewId, u64>,
    clip_seeded: Option<String>,
    pub library_tab: crate::ui::panels::LibraryTab,
    pub library_search: Option<Entity<gpui_kit::component::input::InputState>>,
    pub settings_inputs: Option<crate::ui::settings::SettingsInputs>,

    pub focus_handle: FocusHandle,
    window_handle: AnyWindowHandle,
    saver: Saver,
    /// `MORI_START_URL` sessions are throwaway: the real session file is
    /// never overwritten.
    ephemeral_session: bool,
    save_scheduled: bool,
    last_maintenance: Instant,
    _subscriptions: Vec<Subscription>,
}

impl Browser {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let data_dir = mori_core::paths::data_dir();
        let settings = Settings::load(&data_dir);
        let mut ws = Workspace::load(&data_dir);
        let start_url = std::env::var("MORI_START_URL").ok();
        if let Some(url) = &start_url {
            ws = Workspace::new();
            ws.add_new_tab(TabRecord::new(url.clone()));
        }

        let downloads = DownloadStore::load(&data_dir);
        let zoom = ZoomStore::load(&data_dir);
        let saver = Saver::new(&data_dir);
        let mut this = Browser {
            history: HistoryStore::load(&data_dir),
            bookmarks: BookmarkStore::load(&data_dir),
            archive: ArchiveStore::load(&data_dir),
            routes: RouteStore::load(&data_dir),
            boosts: BoostStore::load(&data_dir),
            site_permissions: SitePermissionStore::load(&data_dir),
            favicons: Favicons::new(data_dir.join("favicons")),
            ws,
            settings,
            live: HashMap::new(),
            downloads,
            downloads_dir: mori_core::paths::downloads_dir(),
            download_tabs: HashMap::new(),
            zoom,
            devtools: HashMap::new(),
            popup: None,
            toasts: Vec::new(),
            next_toast: 1,
            keyed_toasts: HashMap::new(),
            window_visible: true,
            launcher: None,
            launcher_events: None,
            find_bar: None,
            menu: None,
            panel: None,
            settings_open: false,
            peek: None,
            dialogs: PromptQueue::default(),
            dialog_focus: cx.focus_handle(),
            auth_prompts: PromptQueue::default(),
            permissions: PromptQueue::default(),
            cert_errors: PromptQueue::default(),
            cert_exceptions: Default::default(),
            alt_click_at: None,
            rename: None,
            rename_events: None,
            boost_editor: None,
            boost_events: None,
            zapping: None,
            sidebar_peek: false,
            sidebar_hide: None,
            sidebar_drag: None,
            multi_select: Vec::new(),
            assistant: None,
            assistant_visible: false,
            split: None,
            split_focused: false,
            split_drag: None,
            child_anchor: None,
            typed_https: HashMap::new(),
            pending_scroll: HashMap::new(),
            media: HashMap::new(),
            media_checks: Default::default(),
            clip_seq: HashMap::new(),
            clip_seeded: None,
            library_tab: Default::default(),
            library_search: None,
            settings_inputs: None,
            focus_handle: cx.focus_handle(),
            window_handle: window.window_handle(),
            saver,
            ephemeral_session: start_url.is_some(),
            save_scheduled: false,
            last_maintenance: Instant::now(),
            _subscriptions: Vec::new(),
        };
        this.settings.sidebar_visible = this.settings.launch_sidebar_visible();

        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            this.apply_theme(Some(window), cx);
        });
        this._subscriptions.push(appearance);
        this.window_visible = window.is_visible();
        let visibility = cx.observe_window_visibility(window, |this, visibility, _, cx| {
            this.window_visible = visibility.is_visible();
            this.sync_view_visibility(cx);
        });
        this._subscriptions.push(visibility);
        this.watch_downloads(cx);
        // Persist on quit and when the window goes away.
        this._subscriptions.push(cx.on_app_quit(|this, _| {
            this.save_and_flush();
            async {}
        }));
        this._subscriptions.push(cx.on_release(|this, _| this.save_and_flush()));
        this.apply_theme(Some(window), cx);

        if this.ws.tabs.is_empty() {
            this.open_blank_tab(window, cx);
        } else {
            let selected = this
                .ws
                .selected()
                .or_else(|| this.ws.active().visible().first().copied());
            if let Some(id) = selected {
                this.select_tab(id, window, cx);
            }
        }
        this.start_maintenance(cx);
        this.start_media_polling(cx);
        this
    }

    // ---- Persistence ---------------------------------------------------------

    /// Coalesce saves: write the session on the next tick.
    pub fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if self.save_scheduled {
            return;
        }
        self.save_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            this.update(cx, |this, _| {
                this.save_scheduled = false;
                this.save_now();
            })
            .ok();
        })
        .detach();
    }

    /// Queue every store that changed since its last save. Serializing
    /// happens here; the atomic writes run on the saver's own thread.
    pub fn save_now(&mut self) {
        use mori_core::{
            archive, bookmarks, boosts, downloads, history, routes, settings, site_permissions, workspace, zoom,
        };
        let saver = &mut self.saver;
        let session = if self.ephemeral_session {
            Ok(false)
        } else {
            saver.stage(workspace::SESSION_FILE, &self.ws)
        };
        let results = [
            ("session", session),
            ("settings", saver.stage(settings::SETTINGS_FILE, &self.settings)),
            ("history", saver.stage(history::HISTORY_FILE, &self.history)),
            ("bookmarks", saver.stage(bookmarks::BOOKMARKS_FILE, &self.bookmarks)),
            ("archive", saver.stage(archive::ARCHIVE_FILE, &self.archive)),
            ("routes", saver.stage(routes::ROUTES_FILE, &self.routes)),
            ("boosts", saver.stage(boosts::BOOSTS_FILE, &self.boosts)),
            ("downloads", saver.stage(downloads::DOWNLOADS_FILE, &self.downloads)),
            ("zoom", saver.stage(zoom::ZOOM_FILE, &self.zoom)),
            (
                "site permissions",
                saver.stage(site_permissions::SITE_PERMISSIONS_FILE, &self.site_permissions),
            ),
        ];
        for (what, result) in results {
            if let Err(e) = result {
                log::error!("saving {what}: {e:#}");
            }
        }
    }

    /// Save and wait until everything is on disk (quit, window closed).
    pub fn save_and_flush(&mut self) {
        self.save_now();
        self.saver.flush();
    }

    /// Keep a subscription alive for the lifetime of the browser.
    pub fn push_subscription(&mut self, sub: Subscription) {
        self._subscriptions.push(sub);
    }

    pub fn apply_theme(&mut self, window: Option<&mut Window>, cx: &mut App) {
        let gradient = self.ws.active().theme.clone();
        crate::theme::apply(self.settings.appearance, &gradient, window, cx);
    }

    // ---- Toasts --------------------------------------------------------------

    pub fn toast(
        &mut self,
        text: impl Into<SharedString>,
        style: ToastStyle,
        action: Option<(SharedString, ToastAction)>,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_toast;
        self.next_toast += 1;
        let expires = action.is_some();
        self.toasts.push(Toast {
            id,
            text: text.into(),
            style,
            action,
            leaving: false,
            serial: 0,
        });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
        cx.notify();
        self.expire_toast(id, 0, expires, cx);
    }

    /// Dismiss toast `id` after its display time, unless it was updated in
    /// place (its serial changed) meanwhile.
    pub(crate) fn expire_toast(&mut self, id: u64, serial: u64, has_action: bool, cx: &mut Context<Self>) {
        let ttl = if has_action {
            Duration::from_secs(5)
        } else {
            Duration::from_millis(2400)
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ttl).await;
            this.update(cx, |this, cx| {
                if this.toasts.iter().any(|t| t.id == id && t.serial == serial) {
                    this.dismiss_toast(id, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Fade a toast out, then remove it.
    pub fn dismiss_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(toast) = self.toasts.iter_mut().find(|t| t.id == id && !t.leaving) else {
            return;
        };
        toast.leaving = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_EXIT).await;
            this.update(cx, |this, cx| {
                this.toasts.retain(|t| t.id != id);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn run_toast_action(
        &mut self,
        id: u64,
        action: ToastAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_toast(id, cx);
        match action {
            ToastAction::UndoClose(closed) => {
                let tab = self.ws.undo_close(*closed);
                self.select_tab(tab, window, cx);
            }
            ToastAction::UndoCloseMany(closed) => {
                // Reverse closing order puts every tab back at its old index.
                for c in closed.into_iter().rev() {
                    self.ws.undo_close(c);
                }
                self.schedule_save(cx);
            }
            ToastAction::UndoArchive { closed, entry } => {
                self.archive.remove(entry);
                let tab = self.ws.restore_tab(*closed);
                self.select_tab(tab, window, cx);
            }
            ToastAction::UndoDeleteSpace(deleted) => {
                let (deleted, routes) = *deleted;
                let space = self.ws.restore_space(deleted);
                self.routes.restore(routes);
                self.switch_space(space, window, cx);
                self.schedule_save(cx);
            }
            ToastAction::UndoClearHistory(cleared) => {
                self.history.restore(*cleared);
                self.schedule_save(cx);
            }
            ToastAction::Unpin(tab) => {
                self.ws.unpin(tab);
                self.schedule_save(cx);
            }
            ToastAction::OpenDownloads => self.panel = Some(Panel::Downloads),
            ToastAction::Reveal(path) => crate::ui::panels::reveal(&path),
            ToastAction::ResetZoom => self.zoom(0.0, cx),
            ToastAction::PrintTab(tab) => self.print_tab(tab, cx),
            ToastAction::PrintPeek => {
                if let Some(peek) = &self.peek {
                    let (view, title) = (peek.view.clone(), peek.title.clone());
                    self.print_view(view, title, cx);
                }
            }
        }
        cx.notify();
    }

    // ---- Tabs ------------------------------------------------------------------

    pub fn selected(&self) -> Option<TabId> {
        self.ws.selected()
    }

    /// The selected tab's page: the main (left) web card.
    pub fn selected_live(&self) -> Option<&LiveTab> {
        self.selected().and_then(|id| self.live.get(&id))
    }

    /// The tab whose page has the user's attention: the focused split pane,
    /// else the selected tab. Toolbar buttons, the omnibox, Ctrl+W and page
    /// shortcuts act on it.
    pub fn active_tab(&self) -> Option<TabId> {
        match self.split {
            Some(s) if self.split_focused && self.live.contains_key(&s) => Some(s),
            _ => self.selected(),
        }
    }

    pub fn active_live(&self) -> Option<&LiveTab> {
        self.active_tab().and_then(|id| self.live.get(&id))
    }

    /// The page that page commands (back, reload, find, print…) go to: the
    /// active pane's view.
    pub fn selected_view(&self) -> Option<WebView> {
        self.active_live().map(|l| l.view.clone())
    }

    pub fn is_internal(url: &str) -> bool {
        url.is_empty() || url.starts_with("mori://") || url == "about:blank"
    }

    /// Internal pages that show Mori's new-tab page (`about:blank` stays a
    /// plain blank page, for "New tab opens: Blank page").
    pub fn shows_new_tab_page(url: &str) -> bool {
        url.is_empty() || url.starts_with("mori://")
    }

    /// Size of the web card in logical pixels, for creating views at the
    /// right size before their first layout.
    fn card_size(&self, window: &Window) -> (i32, i32) {
        let b = window.viewport_size();
        let sidebar = if self.settings.sidebar_visible {
            self.settings.clamped_sidebar_width()
        } else {
            0.
        };
        (
            (f32::from(b.width) - sidebar - 16.).max(100.) as i32,
            (f32::from(b.height) - 16.).max(100.) as i32,
        )
    }

    /// Load a tab's page in Chromium if it isn't already.
    pub fn realize(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if self.live.contains_key(&id) {
            return;
        }
        let Some(record) = self.ws.tab(id) else { return };
        if Self::is_internal(&record.url) {
            return;
        }
        let view = WebView::new(&record.url, self.card_size(window), window.scale_factor());
        self.adopt_view(id, view, window, cx);
    }

    fn adopt_view(&mut self, id: TabId, view: WebView, window: &mut Window, cx: &mut Context<Self>) {
        let surface = cx.new(|cx| WebSurface::new(view.clone(), cx));
        let surface_events = self.watch_surface(&surface, cx);
        let scroll_y = self.ws.tab(id).map(|t| t.scroll_y).unwrap_or_default();
        if scroll_y > 0.0 {
            self.pending_scroll.insert(id, scroll_y);
        }
        let weak = cx.entity().downgrade();
        let handle = self.window_handle;
        EngineHub::register(cx, &surface, move |_, events, cx| {
            let weak = weak.clone();
            handle
                .update(cx, |_, window, cx| {
                    weak.update(cx, |this, cx| this.on_view_events(id, events, window, cx))
                        .ok();
                })
                .ok();
        });
        let selected = self.selected() == Some(id);
        view.set_hidden(!selected);
        // A view adopted from a popup or Peek has already loaded and will
        // not report its loading state again.
        let loading = view.is_loading().unwrap_or(true);
        let (can_go_back, can_go_forward) = (view.can_go_back(), view.can_go_forward());
        self.live.insert(
            id,
            LiveTab {
                surface,
                view,
                loading,
                progress: 0.0,
                can_go_back,
                can_go_forward,
                error: None,
                find_count: 0,
                find_active: 0,
                reader: false,
                fullscreen: false,
                status: None,
                muted: false,
                opener: None,
                blank_popup: false,
                close_requested: None,
                _surface_events: surface_events,
            },
        );
        if selected {
            self.focus_selected_page(window, cx);
        }
    }

    /// In a split view, the pane whose page holds keyboard focus (after a
    /// click into it) becomes the active one.
    fn sync_split_focus(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(split) = self.split else { return };
        // A pane's DevTools counts as that pane.
        let has_focus = |tab: Option<TabId>| {
            tab.is_some_and(|t| {
                self.live.get(&t).is_some_and(|l| l.surface.read(cx).focus_handle().is_focused(window))
                    || self.devtools.get(&t).is_some_and(|p| p.surface.read(cx).focus_handle().is_focused(window))
            })
        };
        let focused = if has_focus(Some(split)) {
            true
        } else if has_focus(self.selected()) {
            false
        } else {
            return;
        };
        if self.split_focused != focused {
            self.split_focused = focused;
            cx.notify();
        }
    }

    /// Drop a tab's Chromium page but keep the tab (sleep).
    pub fn unload(&mut self, id: TabId, cx: &mut Context<Self>) {
        if self.split == Some(id) {
            self.split = None;
            self.split_focused = false;
        }
        if let Some(live) = self.live.remove(&id) {
            EngineHub::unregister(cx, live.view.id());
            live.view.close();
        }
        self.drop_devtools(id, cx);
        self.drop_popup_of(id, cx);
        // Closing the view already answered its callbacks.
        self.forget_prompts(PageOwner::Tab(id));
        self.pending_scroll.remove(&id);
        cx.notify();
    }

    /// Select a tab. A tab in a split pair shows the pair (its left tab
    /// selected) and makes the clicked tab's pane the active one.
    pub fn select_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let previous_space = self.ws.active_space;
        let previous = [self.selected(), self.split];
        let previous_selected = previous[0];
        let (left, right) = match self.ws.split_of(id) {
            Some(pair) => (pair.tabs[0], Some(pair.tabs[1])),
            None => (id, None),
        };
        if !self.ws.select(left) {
            return;
        }
        // Tabs leaving the screen were in use until now.
        let now = mori_core::paths::now();
        for tab in previous.into_iter().flatten().chain([id]) {
            if let Some(t) = self.ws.tab_mut(tab) {
                t.last_accessed = now;
            }
        }
        if previous_space != self.ws.active_space {
            self.apply_theme(Some(window), cx);
        }
        self.settings_open = false;
        self.multi_select.clear();
        self.child_anchor = None;
        self.realize(left, window, cx);
        self.split = None;
        if let Some(right) = right {
            self.realize(right, window, cx);
            if self.live.contains_key(&right) {
                self.split = Some(right);
            } else {
                // Its page can't be shown (an internal page): no ghost pane.
                self.ws.remove_split(right);
            }
        }
        self.split_focused = self.split.is_some() && self.split == Some(id);
        let split = self.split;
        for (tab, live) in &self.live {
            let hidden = *tab != left && Some(*tab) != split;
            live.view.set_hidden(hidden);
            if hidden {
                live.surface.update(cx, |s, _| s.blur());
            }
        }
        for tab in previous.into_iter().flatten() {
            if tab != left && Some(tab) != split {
                self.capture_scroll(tab, cx);
            }
        }
        if let Some(find) = &self.find_bar
            && previous_selected != Some(id)
        {
            if let Some(prev) = previous_selected.and_then(|p| self.live.get(&p)) {
                prev.view.stop_finding(true);
            }
            find.update(cx, |f, cx| f.rerun(cx));
        }
        self.focus_selected_page(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Give keyboard focus to the active page (the focused split pane or the
    /// selected tab), or to the window when it shows no page.
    pub fn focus_selected_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launcher.is_some() || self.rename.is_some() {
            return;
        }
        if self.settings_open {
            // The settings page covers the pages; keys go to the window.
            window.focus(&self.focus_handle, cx);
            return;
        }
        // A page's open dialog takes the keyboard instead of the page.
        if let Some(owner) = self.active_tab().map(PageOwner::Tab)
            && let Some(handle) = self.modal_focus(owner, cx)
        {
            window.focus(&handle, cx);
            return;
        }
        match self.active_live() {
            Some(live) => {
                let handle = live.surface.read(cx).focus_handle().clone();
                window.focus(&handle, cx);
            }
            None => window.focus(&self.focus_handle, cx),
        }
    }

    /// Focus the active page once the current event has been handled, for
    /// callers without a window.
    fn focus_page_later(&self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let handle = self.window_handle;
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, cx| {
                    weak.update(cx, |this, cx| this.focus_selected_page(window, cx)).ok();
                })
                .ok();
        });
    }

    /// A click anywhere in the window has been handled. A click into a split
    /// pane makes it the active pane. A click on plain chrome (toolbar,
    /// sidebar background, space header) focuses the window itself; give the
    /// keyboard back to the page unless something that takes typing is open.
    pub fn after_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_split_focus(window, cx);
        let typing_elsewhere = self.launcher.is_some()
            || self.rename.is_some()
            || self.settings_open
            || self.menu.is_some()
            || self.boost_editor.is_some()
            || self.peek.is_some()
            || self.popup.is_some()
            || !self.dialogs.is_empty()
            || !self.auth_prompts.is_empty();
        if !typing_elsewhere && self.focus_handle.is_focused(window) && self.active_live().is_some() {
            self.focus_selected_page(window, cx);
        }
    }

    /// Add a tab at the top of the active space's tab list and select it.
    pub fn new_tab_with_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) -> TabId {
        let id = self.ws.add_new_tab(TabRecord::new(url));
        self.select_tab(id, window, cx);
        id
    }

    /// Open a fresh tab as "New tab opens" says. A blank one also opens the
    /// launcher, so typing right away goes somewhere.
    pub fn open_blank_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> TabId {
        let url = self.settings.new_tab_url().to_string();
        let id = self.new_tab_with_url(&url, window, cx);
        if Self::is_internal(&url) {
            cx.defer_in(window, move |this, window, cx| {
                let still_blank = this.selected() == Some(id)
                    && this.ws.tab(id).is_some_and(|t| Self::is_internal(&t.url));
                let busy = this.launcher.is_some() || this.rename.is_some() || this.settings_open;
                if still_blank && !busy {
                    this.open_launcher(LauncherMode::NewTab, window, cx);
                }
            });
        }
        id
    }

    /// Open user input (URL or search) in a new tab or the active one.
    pub fn open_input(&mut self, input: &str, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (url, upgraded) = self.resolve_typed(input);
        let id = self.open_url_in_tab(&url, new_tab, window, cx);
        if upgraded {
            self.typed_https.insert(id, url);
        }
    }

    /// The URL for typed input, and whether Mori upgraded it to https (so a
    /// failed load retries over http). LAN addresses open over http.
    fn resolve_typed(&self, input: &str) -> (String, bool) {
        match omnibox::resolve(input, &self.settings) {
            Resolved::Url(u) => {
                let url = navigation::typed_url(input, &u);
                let upgraded = navigation::is_upgraded(input, &url);
                (url, upgraded)
            }
            Resolved::Search(u) => (u, false),
        }
    }

    pub fn open_url(&mut self, url: &str, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.open_url_in_tab(url, new_tab, window, cx);
    }

    /// [`Browser::open_url`], returning the tab that loads it.
    fn open_url_in_tab(&mut self, url: &str, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) -> TabId {
        // An empty "New Tab" page is reused rather than left behind.
        let active = self.active_tab();
        let active_is_blank = active
            .and_then(|id| self.ws.tab(id))
            .is_some_and(|t| Self::is_internal(&t.url) && !self.settings_open);
        match active {
            Some(id) if !new_tab || active_is_blank => {
                self.navigate(id, url, window, cx);
                id
            }
            _ => self.new_tab_with_url(url, window, cx),
        }
    }

    /// Open links handed to Mori from outside (command line, another app).
    /// Air Traffic Control sends each into the space its rule names; the
    /// first link is selected and the rest follow it in order.
    pub fn open_external_urls(&mut self, inputs: &[String], window: &mut Window, cx: &mut Context<Self>) {
        if inputs.is_empty() {
            return;
        }
        // The command bar a blank start tab opened yields to the handed-off
        // pages (it would otherwise sit over the page that replaced it).
        if self
            .launcher
            .as_ref()
            .is_some_and(|l| l.read(cx).mode() == LauncherMode::NewTab && l.read(cx).query(cx).is_empty())
        {
            self.close_launcher(window, cx);
        }
        let mut first: Option<TabId> = None;
        let mut last_in_space: HashMap<SpaceId, TabId> = HashMap::new();
        for input in inputs {
            let (url, upgraded) = self.resolve_typed(input);
            let space = self
                .routes
                .match_url(&url)
                .map(|r| r.context_id)
                .filter(|s| self.ws.space(*s).is_some())
                .unwrap_or(self.ws.active_space);
            let reusable = self
                .selected()
                .filter(|_| first.is_none() && space == self.ws.active_space && !self.settings_open)
                .filter(|id| self.ws.tab(*id).is_some_and(|t| Self::is_internal(&t.url)));
            let id = match reusable {
                Some(id) => {
                    self.navigate(id, &url, window, cx);
                    id
                }
                None => {
                    let mut record = TabRecord::new(url.clone());
                    // Shown until the page loads (only the first one does now).
                    record.title = omnibox::display_url(&url);
                    match last_in_space.get(&space) {
                        Some(prev) => self.ws.add_child_tab(record, *prev, None),
                        None => self.ws.add_tab(record, Some(space), Slot::Loose(0)),
                    }
                }
            };
            if upgraded {
                self.typed_https.insert(id, url);
            }
            last_in_space.insert(space, id);
            first.get_or_insert(id);
        }
        let Some(first) = first else { return };
        let routed = self.ws.space_of(first).filter(|s| *s != self.ws.active_space);
        self.select_tab(first, window, cx);
        if let Some(space) = routed.and_then(|s| self.ws.space(s)) {
            let name = space.name.clone();
            self.toast(format!("Opened in {name}"), ToastStyle::Info, None, cx);
        }
        self.schedule_save(cx);
    }

    pub fn navigate(&mut self, id: TabId, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.typed_https.remove(&id);
        // A new address starts at the top, not at the old page's offset.
        self.pending_scroll.remove(&id);
        if self.settings_open && self.active_tab() == Some(id) {
            self.set_settings_open(false, window, cx);
        }
        if let Some(t) = self.ws.tab_mut(id) {
            t.url = url.to_string();
            t.scroll_y = 0.0;
        }
        // A credentials or certificate question about the old page is moot.
        self.cancel_navigation_prompts(PageOwner::Tab(id), cx);
        if let Some(live) = self.live.get_mut(&id) {
            live.error = None;
            live.reader = false;
            if Self::is_internal(url) {
                // A split pane can only show web pages.
                if self.split == Some(id) {
                    self.ws.remove_split(id);
                }
                self.unload(id, cx);
            } else {
                live.view.load_url(url);
            }
        } else {
            self.realize(id, window, cx);
        }
        self.focus_selected_page(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Close (or, per Mori rules, keep) a tab. `force` is used by the close
    /// button and menu, which really close pinned/foldered tabs too.
    pub fn close_tab(&mut self, id: TabId, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !force && self.ws.is_pinned(id) {
            self.toast(
                "Pinned tab stays put",
                ToastStyle::Info,
                Some(("Unpin".into(), ToastAction::Unpin(id))),
                cx,
            );
            return;
        }
        if !force && self.ws.folder_of(id).is_some() {
            // Unloading a split pane ends its pair; otherwise selecting the
            // neighbour (often its partner) would load it straight back.
            if self.ws.remove_split(id) && self.visible_split_member(id) {
                self.split = None;
                self.split_focused = false;
                let visible = self.selected();
                for (tab, live) in &self.live {
                    live.view.set_hidden(Some(*tab) != visible);
                }
            }
            let was_selected = self.selected() == Some(id);
            let neighbor = self.ws.neighbor(id);
            self.unload(id, cx);
            if was_selected {
                match neighbor {
                    Some(n) => self.select_tab(n, window, cx),
                    None => {
                        self.open_blank_tab(window, cx);
                    }
                }
            } else {
                self.focus_selected_page(window, cx);
            }
            self.toast("Unloaded — kept in folder", ToastStyle::Info, None, cx);
            return;
        }
        // Let the page run its beforeunload handler (it may ask "Leave
        // site?"); the tab closes when Chromium reports the page agreed
        // (`ViewEvent::CloseRequested`). Asking again while the page is still
        // answering does nothing, so a quick second Ctrl+W can't skip the
        // page's handler; asking again once it has stopped answering closes
        // it outright.
        if let Some(live) = self.live.get_mut(&id)
            && !live.view.is_closed()
        {
            match live.close_requested {
                None => {
                    live.close_requested = Some(Instant::now());
                    live.view.request_close();
                    return;
                }
                Some(at) if at.elapsed() < CLOSE_ANSWER_TIME => return,
                Some(_) => {}
            }
        }
        if self.dialogs.front(PageOwner::Tab(id)).is_some_and(|d| d.dialog.kind == DialogKind::BeforeUnload) {
            return;
        }
        self.remove_closed_tab(id, window, cx);
    }

    /// Take a tab out of the space for good (with Undo), once its page has
    /// agreed to close or was forced to.
    fn remove_closed_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let was_selected = self.selected() == Some(id);
        let Some(closed) = self.take_tab(id, true, cx) else { return };
        let title: String = closed.record.display_title().chars().take(40).collect();
        self.toast(
            format!("Closed {title}"),
            ToastStyle::Info,
            Some(("Undo".into(), ToastAction::UndoClose(Box::new(closed)))),
            cx,
        );
        self.after_tabs_closed(was_selected, window, cx);
    }

    /// Unload a tab and remove it from the workspace; `reopenable` puts it
    /// on the recently-closed stack.
    fn take_tab(&mut self, id: TabId, reopenable: bool, cx: &mut Context<Self>) -> Option<ClosedTab> {
        self.unload(id, cx);
        self.multi_select.retain(|t| *t != id);
        self.typed_https.remove(&id);
        if reopenable {
            self.ws.close_tab(id)
        } else {
            self.ws.remove_tab(id)
        }
    }

    /// After closing tabs: keep at least one tab in the space, select a
    /// neighbour if the selected tab went away, and give the keyboard back
    /// to the page.
    fn after_tabs_closed(&mut self, was_selected: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.ws.active().ordered().is_empty() {
            self.open_blank_tab(window, cx);
        } else if was_selected {
            let next = self
                .ws
                .selected()
                .or_else(|| self.ws.active().visible().first().copied());
            if let Some(next) = next {
                self.select_tab(next, window, cx);
            }
        } else {
            self.focus_selected_page(window, cx);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Close several tabs with one toast whose Undo brings them all back.
    pub fn close_tabs(&mut self, ids: Vec<TabId>, window: &mut Window, cx: &mut Context<Self>) {
        let was_selected = self.selected().is_some_and(|s| ids.contains(&s));
        let closed: Vec<ClosedTab> = ids
            .into_iter()
            .filter_map(|id| self.take_tab(id, true, cx))
            .collect();
        match closed.len() {
            0 => return,
            1 => {
                let title: String = closed[0].record.display_title().chars().take(40).collect();
                let closed = closed.into_iter().next().map(Box::new);
                self.toast(
                    format!("Closed {title}"),
                    ToastStyle::Info,
                    closed.map(|c| ("Undo".into(), ToastAction::UndoClose(c))),
                    cx,
                );
            }
            n => self.toast(
                format!("Closed {n} tabs"),
                ToastStyle::Info,
                Some(("Undo".into(), ToastAction::UndoCloseMany(closed))),
                cx,
            ),
        }
        self.after_tabs_closed(was_selected, window, cx);
    }

    pub fn reopen_closed_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.ws.reopen_closed() {
            // Its toast's Undo would only point at an open tab now.
            self.toasts.retain(
                |t| !matches!(&t.action, Some((_, ToastAction::UndoClose(c))) if c.record.id == id),
            );
            self.select_tab(id, window, cx);
        }
    }

    pub fn duplicate_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.ws.tab(id).cloned() else { return };
        let mut copy = TabRecord::new(record.url);
        copy.title = record.title;
        copy.favicon_url = record.favicon_url;
        let new_id = self.ws.add_child_tab(copy, id, None);
        self.select_tab(new_id, window, cx);
    }

    /// Close every loose tab of the space except `keep` (pins and folders
    /// are saved collections and stay).
    pub fn close_other_tabs(&mut self, keep: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let others: Vec<TabId> = self
            .ws
            .active()
            .loose
            .iter()
            .copied()
            .filter(|t| *t != keep)
            .collect();
        self.close_tabs(others, window, cx);
    }

    pub fn close_tabs_below(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let loose = self.ws.active().loose.clone();
        if let Some(i) = loose.iter().position(|t| *t == id) {
            self.close_tabs(loose[i + 1..].to_vec(), window, cx);
        }
    }

    /// Tabs whose page is on screen.
    fn visible_tabs(&self) -> [Option<TabId>; 2] {
        [self.selected(), self.split]
    }

    /// Whether `id` is one of the two panes of the split view on screen.
    pub fn visible_split_member(&self, id: TabId) -> bool {
        self.split.is_some() && self.visible_tabs().contains(&Some(id))
    }

    pub fn sleep_tab(&mut self, id: TabId, cx: &mut Context<Self>) {
        if !self.visible_tabs().contains(&Some(id)) {
            self.unload(id, cx);
        }
    }

    pub fn sleep_background_tabs(&mut self, cx: &mut Context<Self>) {
        let visible = self.visible_tabs();
        let ids: Vec<TabId> = self
            .live
            .keys()
            .copied()
            .filter(|t| !visible.contains(&Some(*t)))
            .collect();
        let n = ids.len();
        for id in ids {
            self.unload(id, cx);
        }
        self.toast(
            format!("Put {n} tab{} to sleep", if n == 1 { "" } else { "s" }),
            ToastStyle::Success,
            None,
            cx,
        );
    }

    /// Close a tab into the archive, with an Undo that reopens it and drops
    /// the archive entry again.
    pub fn archive_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if self.ws.is_pinned(id) {
            return;
        }
        let entry = self
            .ws
            .tab(id)
            .and_then(|t| self.archive.add(&t.url, &t.title, t.favicon_url.clone()));
        let Some(entry) = entry else {
            // Blank and internal pages have nothing worth archiving.
            self.close_tab(id, true, window, cx);
            return;
        };
        let was_selected = self.selected() == Some(id);
        let Some(closed) = self.take_tab(id, false, cx) else { return };
        let title: String = closed.record.display_title().chars().take(40).collect();
        self.toast(
            format!("Archived {title}"),
            ToastStyle::Info,
            Some((
                "Undo".into(),
                ToastAction::UndoArchive {
                    closed: Box::new(closed),
                    entry,
                },
            )),
            cx,
        );
        self.after_tabs_closed(was_selected, window, cx);
    }

    pub fn toggle_pin(&mut self, id: TabId, cx: &mut Context<Self>) {
        self.ws.toggle_pin(id);
        self.schedule_save(cx);
        cx.notify();
    }

    pub fn toggle_mute(&mut self, id: TabId, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get_mut(&id) {
            live.muted = !live.muted;
            live.view.set_audio_muted(live.muted);
        }
        cx.notify();
    }

    pub fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.ws.cycle_from(self.active_tab(), delta) {
            self.select_tab(id, window, cx);
        }
    }

    pub fn select_ordinal(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.ws.by_ordinal(n) {
            self.select_tab(id, window, cx);
        }
    }

    // ---- Split view ------------------------------------------------------------

    /// Show `tab` beside the selected tab as a remembered split pair, with
    /// the new pane active.
    pub fn open_split(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(left) = self.selected() else { return };
        if tab == left || self.ws.space_of(tab) != Some(self.ws.active_space) {
            return;
        }
        if self.ws.tab(tab).is_none_or(|t| Self::is_internal(&t.url)) {
            self.toast("Only web pages can open in split view", ToastStyle::Info, None, cx);
            return;
        }
        if self.ws.create_split(left, tab) {
            self.select_tab(tab, window, cx);
        }
    }

    /// End the split view on screen; the selected tab stays.
    pub fn close_split(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.split.take() {
            self.ws.remove_split(tab);
            self.split_focused = false;
            self.split_drag = None;
            if let Some(live) = self.live.get(&tab) {
                live.view.set_hidden(true);
                live.surface.update(cx, |s, _| s.blur());
            }
            self.focus_page_later(cx);
            self.schedule_save(cx);
            cx.notify();
        }
    }

    /// The left pane's share of the web area for the split on screen.
    pub fn split_ratio(&self) -> f32 {
        self.split_drag.unwrap_or_else(|| {
            self.split
                .and_then(|s| self.ws.split_of(s))
                .map(|g| g.clamped_ratio())
                .unwrap_or(0.5)
        })
    }

    /// Divider being dragged: follow the pointer without saving yet.
    pub fn drag_split_divider(&mut self, ratio: f32, cx: &mut Context<Self>) {
        if self.split.is_some() {
            let min = mori_core::workspace::SPLIT_MIN_RATIO;
            self.split_drag = Some(ratio.clamp(min, 1.0 - min));
            cx.notify();
        }
    }

    /// Divider released (or double-clicked with `reset`): keep the ratio.
    pub fn finish_split_divider(&mut self, reset: bool, cx: &mut Context<Self>) {
        let ratio = if reset { Some(0.5) } else { self.split_drag.take() };
        self.split_drag = None;
        if let (Some(tab), Some(ratio)) = (self.split, ratio) {
            self.ws.set_split_ratio(tab, ratio);
            self.schedule_save(cx);
            cx.notify();
        }
    }

    // ---- Spaces --------------------------------------------------------------

    pub fn switch_space(&mut self, space: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ws.switch_space(space) {
            return;
        }
        self.apply_theme(Some(window), cx);
        let target = self
            .ws
            .active()
            .selected
            .or_else(|| self.ws.active().visible().first().copied());
        match target {
            Some(id) => self.select_tab(id, window, cx),
            None => {
                self.open_blank_tab(window, cx);
            }
        }
        cx.notify();
    }

    pub fn create_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.ws.next_space_name();
        let id = self.ws.create_space(name);
        self.switch_space(id, window, cx);
        self.begin_rename(RenameTarget::Space(id), window, cx);
    }

    /// Delete a space with all its tabs; the toast's Undo brings it back
    /// with its tabs, folders, pins and Air Traffic Control rules.
    pub fn delete_space(&mut self, space: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        if self.ws.spaces.len() <= 1 {
            self.toast("Can't delete the last space", ToastStyle::Warning, None, cx);
            return;
        }
        let ids = self.ws.space(space).map(|s| s.ordered()).unwrap_or_default();
        for id in &ids {
            self.unload(*id, cx);
        }
        let Some(deleted) = self.ws.delete_space(space) else { return };
        let routes = self.routes.take_for_context(space);
        let name = deleted.space.name.clone();
        let text = match deleted.tabs.len() {
            0 => format!("Deleted {name}"),
            1 => format!("Deleted {name} and 1 tab"),
            n => format!("Deleted {name} and {n} tabs"),
        };
        self.toast(
            text,
            ToastStyle::Info,
            Some(("Undo".into(), ToastAction::UndoDeleteSpace(Box::new((deleted, routes))))),
            cx,
        );
        let active = self.ws.active_space;
        self.switch_space(active, window, cx);
        self.schedule_save(cx);
    }

    /// Move a tab to another space. If it was the selected tab, a neighbour
    /// in the current space takes over (or a fresh new tab if it was alone).
    pub fn move_tab_to_space(&mut self, tab: TabId, space: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        let on_screen = self.visible_tabs().contains(&Some(tab));
        self.ws.move_to_space(tab, space);
        if on_screen {
            match self.ws.selected().or_else(|| self.ws.active().visible().first().copied()) {
                Some(next) => self.select_tab(next, window, cx),
                None => {
                    self.open_blank_tab(window, cx);
                }
            }
        }
        self.schedule_save(cx);
        cx.notify();
    }

    // ---- Folders & renaming --------------------------------------------------

    pub fn new_folder(&mut self, tabs: Vec<TabId>, window: &mut Window, cx: &mut Context<Self>) {
        let space = self.ws.active_space;
        if let Some(fid) = self.ws.create_folder(space, "New Folder", &tabs) {
            self.multi_select.clear();
            self.begin_rename(RenameTarget::Folder(fid), window, cx);
        }
        self.schedule_save(cx);
    }

    pub fn begin_rename(&mut self, target: RenameTarget, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::input::{InputEvent, InputState};
        let current = match target {
            RenameTarget::Tab(id) => self.ws.tab(id).map(|t| t.display_title().to_string()),
            RenameTarget::Folder(id) => self.ws.find_folder(id).map(|(_, f)| f.name.clone()),
            RenameTarget::Space(id) => self.ws.space(id).map(|s| s.name.clone()),
        }
        .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(current, window, cx);
            state
        });
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        let sub = cx.subscribe_in(&input, window, move |this, input, ev: &InputEvent, window, cx| {
            match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    let value = input.read(cx).value().to_string();
                    this.commit_rename(target, value, window, cx);
                }
                _ => {}
            }
        });
        self.rename_events = Some(sub);
        self.rename = Some((target, input));
        cx.notify();
    }

    fn commit_rename(&mut self, target: RenameTarget, value: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.as_ref().map(|(t, _)| *t) != Some(target) {
            return;
        }
        self.rename = None;
        let value = value.trim().to_string();
        match target {
            RenameTarget::Tab(id) => {
                if let Some(t) = self.ws.tab_mut(id) {
                    t.custom_title = (!value.is_empty() && value != t.title).then_some(value);
                }
            }
            RenameTarget::Folder(id) => {
                let name = if value.is_empty() { "Folder".to_string() } else { value };
                self.ws.update_folder(id, |f| f.name = name);
            }
            RenameTarget::Space(id) => {
                if let Some(s) = self.ws.space_mut(id)
                    && !value.is_empty()
                {
                    s.name = value;
                }
            }
        }
        self.schedule_save(cx);
        self.focus_selected_page(window, cx);
        cx.notify();
    }

    pub fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rename = None;
        self.focus_selected_page(window, cx);
        cx.notify();
    }

    // ---- Launcher ------------------------------------------------------------

    pub fn open_launcher(&mut self, mode: LauncherMode, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(launcher) = &self.launcher {
            let same = launcher.read(cx).mode() == mode;
            if same && mode == LauncherMode::NewTab {
                self.close_launcher(window, cx);
                return;
            }
        }
        self.menu = None;
        self.panel = None;
        let prefill = match mode {
            LauncherMode::NewTab => String::new(),
            LauncherMode::Address => self
                .active_tab()
                .and_then(|id| self.ws.tab(id))
                .map(|t| omnibox::display_url(&t.url))
                .unwrap_or_default(),
        };
        let candidates = crate::ui::launcher::candidates(self);
        let launcher = cx.new(|cx| Launcher::new(mode, prefill, candidates, window, cx));
        let sub = cx.subscribe_in(&launcher, window, |this, _, ev: &LauncherEvent, window, cx| {
            this.on_launcher_event(ev, window, cx);
        });
        self.launcher_events = Some(sub);
        launcher.update(cx, |l, cx| l.focus(window, cx));
        self.launcher = Some(launcher);
        cx.notify();
    }

    pub fn close_launcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launcher.take().is_some() {
            self.focus_selected_page(window, cx);
            cx.notify();
        }
    }

    fn on_launcher_event(&mut self, ev: &LauncherEvent, window: &mut Window, cx: &mut Context<Self>) {
        let new_tab = self
            .launcher
            .as_ref()
            .map(|l| l.read(cx).mode() == LauncherMode::NewTab)
            .unwrap_or(true);
        match ev {
            LauncherEvent::Dismiss => self.close_launcher(window, cx),
            LauncherEvent::OpenInput(text) => {
                self.launcher = None;
                if text.trim().is_empty() {
                    if new_tab {
                        let url = self.settings.new_tab_url().to_string();
                        self.new_tab_with_url(&url, window, cx);
                    } else {
                        self.focus_selected_page(window, cx);
                    }
                } else {
                    self.open_input(text, new_tab, window, cx);
                }
            }
            LauncherEvent::OpenUrl(url) => {
                self.launcher = None;
                self.open_url(url, new_tab, window, cx);
            }
            LauncherEvent::SwitchToTab(id) => {
                self.launcher = None;
                self.select_tab(*id, window, cx);
            }
            LauncherEvent::Command(cmd) => {
                self.launcher = None;
                self.focus_selected_page(window, cx);
                crate::commands::run(self, *cmd, window, cx);
            }
            LauncherEvent::AskAssistant(prompt) => {
                self.launcher = None;
                self.open_assistant(Some(prompt.clone()), window, cx);
            }
        }
        cx.notify();
    }

    // ---- Assistant -------------------------------------------------------------

    pub fn open_assistant(&mut self, prompt: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.ai_integration_enabled {
            self.toast("Turn on AI integration in Settings", ToastStyle::Info, None, cx);
            return;
        }
        let browser = cx.entity().downgrade();
        let panel = self
            .assistant
            .get_or_insert_with(|| {
                cx.new(|cx| crate::ui::assistant::AssistantPanel::new(browser, window, cx))
            })
            .clone();
        self.assistant_visible = true;
        panel.update(cx, |p, cx| {
            p.focus(window, cx);
            if let Some(prompt) = prompt {
                p.send(prompt, window, cx);
            }
        });
        cx.notify();
    }

    /// Turn AI integration on or off. Off closes the assistant panel and
    /// drops it, which shuts down its Codex server.
    pub fn set_ai_integration(&mut self, on: bool) {
        self.settings.ai_integration_enabled = on;
        if !on {
            self.assistant_visible = false;
            self.assistant = None;
        }
    }

    /// Show or hide the settings page. It takes the web card's place, so the
    /// covered pages are hidden and keyboard focus moves to the window (the
    /// covered page can't take keys); closing gives focus back to the page.
    pub fn set_settings_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = open;
        // Pages covered by Settings stop painting.
        self.sync_view_visibility(cx);
        if open {
            window.focus(&self.focus_handle, cx);
        } else {
            self.focus_selected_page(window, cx);
        }
        cx.notify();
    }

    pub fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_settings_open(!self.settings_open, window, cx);
    }

    pub fn toggle_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.assistant_visible {
            self.assistant_visible = false;
            self.focus_selected_page(window, cx);
            cx.notify();
        } else {
            self.open_assistant(None, window, cx);
        }
    }

    // ---- Find ------------------------------------------------------------------

    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_view().is_none() {
            return;
        }
        let bar = match &self.find_bar {
            Some(bar) => bar.clone(),
            None => {
                let bar = cx.new(|cx| FindBar::new(window, cx));
                let sub = cx.subscribe_in(&bar, window, |this, _, ev: &FindBarEvent, window, cx| {
                    let Some(view) = this.selected_view() else { return };
                    match ev {
                        FindBarEvent::Search {
                            text,
                            forward,
                            next,
                            match_case,
                        } => {
                            if text.is_empty() {
                                view.stop_finding(true);
                            } else {
                                view.find(text, *forward, *match_case, *next);
                            }
                        }
                        FindBarEvent::Close => this.close_find(window, cx),
                    }
                });
                // The bar owns its handler, so Ctrl+F again never adds another.
                bar.update(cx, |b, _| b.hold(sub));
                self.find_bar = Some(bar.clone());
                bar
            }
        };
        bar.update(cx, |b, cx| b.focus(window, cx));
        cx.notify();
    }

    pub fn find_step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        match &self.find_bar {
            Some(bar) => bar.update(cx, |b, cx| b.step(forward, cx)),
            None => self.open_find(window, cx),
        }
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_bar.take().is_some() {
            if let Some(view) = self.selected_view() {
                view.stop_finding(true);
            }
            self.focus_selected_page(window, cx);
            cx.notify();
        }
    }

    // ---- Clipboard bridge -----------------------------------------------------------

    /// Listen to a page surface; the subscription lives as long as the
    /// tab, Peek or popup holding it.
    fn watch_surface(&mut self, surface: &Entity<WebSurface>, cx: &mut Context<Self>) -> Subscription {
        use crate::web_surface::SurfaceEvent;
        cx.subscribe(surface, |this, surface, ev: &SurfaceEvent, cx| {
            let view = surface.read(cx).view().clone();
            match ev {
                SurfaceEvent::Interacted => {
                    // A click may also have started or paused media.
                    if let Some(tab) = this.live.iter().find(|(_, l)| l.view == view).map(|(t, _)| *t) {
                        this.check_media_soon(tab, cx);
                    }
                    this.poll_page_clipboard(view, cx)
                }
                SurfaceEvent::PasteRequested { key } => this.paste_key_into(view, key.clone(), cx),
            }
        })
    }

    /// After a user gesture in a page, copy whatever it just copied to the
    /// system clipboard.
    pub fn poll_page_clipboard(&mut self, view: WebView, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            // Let the page's copy handlers run first.
            cx.background_executor().timer(Duration::from_millis(120)).await;
            let async_cx = cx.clone();
            let probe = view.clone();
            cx.update(|_| {
                probe.evaluate(mori_core::scripts::CLIPBOARD_POLL_SCRIPT, move |result| {
                    let Ok(value) = result else { return };
                    let seq = value.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
                    let text = value.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let id = view.id();
                    let _ = weak.update(&mut async_cx.clone(), |this, cx| {
                        let seen = this.clip_seq.insert(id, seq).unwrap_or(0);
                        if seq > seen && !text.is_empty() {
                            this.clip_seeded = Some(text.clone());
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                    });
                });
            });
        })
        .detach();
    }

    /// Paste the system clipboard into a page: seed Chromium's clipboard with
    /// it if it changed, then run the page's paste command.
    pub fn paste_into(&mut self, view: WebView, plain: bool, cx: &mut Context<Self>) {
        self.seed_page_clipboard(view, cx, move |view| {
            if plain {
                view.paste_and_match_style();
            } else {
                view.paste();
            }
        });
    }

    /// The paste shortcut in a page: seed Chromium's clipboard, then send
    /// the key itself, so the page sees its keydown (and may handle it) and
    /// Chromium pastes with genuine paste events otherwise.
    pub fn paste_key_into(&mut self, view: WebView, key: KeyInput, cx: &mut Context<Self>) {
        self.seed_page_clipboard(view, cx, move |view| {
            let mut up = key.clone();
            up.phase = mori_engine::keys::KeyPhase::Up;
            up.repeat = false;
            view.send_key(&key);
            view.send_key(&up);
        });
    }

    fn seed_page_clipboard(
        &mut self,
        view: WebView,
        cx: &mut Context<Self>,
        then: impl FnOnce(&WebView) + 'static,
    ) {
        let text = cx.read_from_clipboard().and_then(|c| c.text());
        match text {
            Some(text) if self.clip_seeded.as_deref() != Some(text.as_str()) => {
                self.clip_seeded = Some(text.clone());
                let script = mori_core::scripts::clipboard_seed_script(&text);
                let target = view.clone();
                // `execCommand('copy')` needs the user's paste gesture.
                view.evaluate_as_user(&script, move |_| then(&target));
            }
            _ => then(&view),
        }
    }

    // ---- Page commands -----------------------------------------------------------

    /// Remember where a page is scrolled, to restore it when the tab's page
    /// is loaded again (after sleep or a restart).
    fn capture_scroll(&self, id: TabId, cx: &mut Context<Self>) {
        let Some(live) = self.live.get(&id) else { return };
        let weak = cx.entity().downgrade();
        let async_cx = cx.to_async();
        live.view.evaluate("window.scrollY", move |result| {
            let Some(y) = result.ok().and_then(|v| v.as_f64()) else { return };
            let _ = weak.update(&mut async_cx.clone(), |this, _| {
                if let Some(t) = this.ws.tab_mut(id) {
                    t.scroll_y = y.max(0.0);
                }
            });
        });
    }

    /// Clear browsing history, with an Undo that brings it back.
    pub fn clear_history(&mut self, cx: &mut Context<Self>) {
        let cleared = std::mem::take(&mut self.history);
        self.toast(
            "History cleared",
            ToastStyle::Success,
            Some(("Undo".into(), ToastAction::UndoClearHistory(Box::new(cleared)))),
            cx,
        );
        self.schedule_save(cx);
    }

    pub fn copy_url(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.active_tab().and_then(|id| self.ws.tab(id))
            && !Self::is_internal(&t.url)
        {
            cx.write_to_clipboard(ClipboardItem::new_string(t.url.clone()));
            self.toast("Copied URL", ToastStyle::Success, None, cx);
        }
    }

    pub fn toggle_bookmark(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.active_tab().and_then(|id| self.ws.tab(id)).cloned() else {
            return;
        };
        if Self::is_internal(&t.url) {
            return;
        }
        let added = self.bookmarks.toggle(&t.url, t.display_title());
        self.toast(
            if added { "Bookmarked" } else { "Removed bookmark" },
            ToastStyle::Success,
            None,
            cx,
        );
        self.schedule_save(cx);
    }

    pub fn toggle_reader(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.active_tab() else { return };
        let Some(live) = self.live.get_mut(&id) else { return };
        if live.reader {
            live.reader = false;
            live.view.reload(false);
            cx.notify();
            return;
        }
        let weak = cx.entity().downgrade();
        let async_cx = cx.to_async();
        live.view.evaluate(mori_core::scripts::READER_SCRIPT, move |result| {
            let applied = matches!(result, Ok(serde_json::Value::Bool(true)));
            let _ = weak.update(&mut async_cx.clone(), |this, cx| {
                if let Some(live) = this.live.get_mut(&id) {
                    live.reader = applied;
                }
                if !applied {
                    this.toast("No article found to read", ToastStyle::Info, None, cx);
                }
                cx.notify();
            });
        });
    }

    pub fn go_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let home = self.settings.homepage_url.clone();
        if let Some(id) = self.active_tab() {
            self.navigate(id, &home, window, cx);
        }
    }

    pub fn peek_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let size = self.peek_view_size(window, cx);
        let view = WebView::new(url, size, window.scale_factor());
        self.peek_view(view, url, window, cx);
    }

    /// Show an already-created view (e.g. a shift-clicked link) in Peek.
    pub fn peek_view(&mut self, view: WebView, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(old) = self.peek.take() {
            EngineHub::unregister(cx, old.view.id());
            old.view.close();
            // The old page's questions must not land on the new one.
            self.forget_prompts(PageOwner::Peek);
            self.sync_page_modals(cx);
        }
        self.menu = None;
        self.launcher = None;
        self.launcher_events = None;
        view.set_hidden(false);
        let surface = cx.new(|cx| WebSurface::new(view.clone(), cx));
        let surface_events = self.watch_surface(&surface, cx);
        let weak = cx.entity().downgrade();
        let handle = self.window_handle;
        EngineHub::register(cx, &surface, move |_, events, cx| {
            let weak = weak.clone();
            handle
                .update(cx, |_, window, cx| {
                    weak.update(cx, |this, cx| this.on_peek_events(events, window, cx)).ok();
                })
                .ok();
        });
        let focus = surface.read(cx).focus_handle().clone();
        window.focus(&focus, cx);
        self.peek = Some(Peek {
            surface,
            view,
            title: String::new(),
            url: url.to_string(),
            _surface_events: surface_events,
            loading: true,
        });
        cx.notify();
    }

    pub fn close_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(peek) = self.peek.take() {
            EngineHub::unregister(cx, peek.view.id());
            peek.view.close();
            self.forget_prompts(PageOwner::Peek);
            self.focus_selected_page(window, cx);
            cx.notify();
        }
    }

    /// Move the peeked page into a new tab after the current one.
    fn adopt_peek(&mut self, peek: Peek, window: &mut Window, cx: &mut Context<Self>) -> TabId {
        EngineHub::unregister(cx, peek.view.id());
        let url = peek.view.url();
        let mut record = TabRecord::new(url.clone());
        record.title = peek.title.clone();
        let id = match self.active_tab() {
            Some(opener) => self.ws.add_child_tab(record, opener, None),
            None => self.ws.add_new_tab(record),
        };
        self.reassign_prompts(PageOwner::Peek, PageOwner::Tab(id));
        // adopt_view reads the page's real loading and history state: a
        // page that finished loading while peeked reports no further loads.
        self.adopt_view(id, peek.view, window, cx);
        if let Some(live) = self.live.get_mut(&id)
            && !live.loading
        {
            live.progress = 1.0;
        }
        if !peek.loading {
            let page = url.split('#').next().unwrap_or(&url).to_string();
            self.history.record(&page, &peek.title);
            self.apply_boost(id);
        }
        id
    }

    /// Turn the peeked page into a real tab in the active space.
    pub fn promote_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek) = self.peek.take() else { return };
        let id = self.adopt_peek(peek, window, cx);
        self.select_tab(id, window, cx);
        self.sync_page_modals(cx);
    }

    /// Turn the peeked page into a tab shown beside the current one.
    pub fn peek_to_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek) = self.peek.take() else { return };
        let id = self.adopt_peek(peek, window, cx);
        self.open_split(id, window, cx);
        self.focus_selected_page(window, cx);
        self.schedule_save(cx);
    }

    fn on_peek_events(&mut self, events: Vec<ViewEvent>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek_view) = self.peek.as_ref().map(|p| p.view.clone()) else { return };
        let mut events = events.into_iter();
        while let Some(ev) = events.next() {
            if self.peek.as_ref().is_none_or(|p| p.view != peek_view) {
                // The Peek became a tab partway through the batch (a popup
                // promoted it): its remaining events, e.g. a dialog, belong
                // to that tab now. A closed Peek's are moot.
                let tab = self.live.iter().find(|(_, l)| l.view == peek_view).map(|(t, _)| *t);
                if let Some(tab) = tab {
                    let rest: Vec<ViewEvent> = std::iter::once(ev).chain(events).collect();
                    self.on_view_events(tab, rest, window, cx);
                }
                return;
            }
            let Some(ev) = self.on_page_event(PageOwner::Peek, ev, window, cx) else { continue };
            let Some(peek) = self.peek.as_mut() else { return };
            match ev {
                ViewEvent::TitleChanged(t) => peek.title = t,
                ViewEvent::LoadingStateChanged { loading, .. } => peek.loading = loading,
                ViewEvent::LoadEnd { .. } => {
                    peek.view.execute_js(mori_core::scripts::CLIPBOARD_BRIDGE_SCRIPT)
                }
                ViewEvent::AddressChanged(u) => peek.url = u,
                ViewEvent::NewView { view, url, disposition, features, .. } => {
                    use mori_engine::Disposition as D;
                    // Keep the new page alive as a tab so window.opener keeps
                    // working. A popup (sign-in flows) needs its opener too,
                    // so the peeked page becomes a tab first.
                    let popup = matches!(disposition, D::Popup | D::Window);
                    match disposition {
                        D::Popup | D::Window => self.promote_peek(window, cx),
                        D::BackgroundTab => {}
                        _ => self.close_peek(window, cx),
                    }
                    let opener = self.active_tab();
                    if popup
                        && let Some(opener) = opener
                        && popups::wants_window(disposition, features)
                    {
                        self.open_popup_window(opener, view, url, features, window, cx);
                        self.schedule_save(cx);
                        continue;
                    }
                    let mut record = TabRecord::new(url.clone());
                    record.title = url;
                    let id = match opener {
                        Some(opener) => self.ws.add_child_tab(record, opener, None),
                        None => self.ws.add_new_tab(record),
                    };
                    self.adopt_view(id, view, window, cx);
                    if popup
                        && let Some(live) = self.live.get_mut(&id)
                    {
                        // window.close() from it returns quietly to the opener.
                        live.opener = opener;
                        live.blank_popup = true;
                    }
                    if disposition == D::BackgroundTab {
                        if let Some(live) = self.live.get(&id) {
                            live.view.set_hidden(true);
                        }
                    } else {
                        self.select_tab(id, window, cx);
                    }
                    self.schedule_save(cx);
                }
                ViewEvent::CloseRequested => self.close_peek(window, cx),
                ViewEvent::FileDialog(request) => {
                    let view = peek.view.clone();
                    self.open_file_dialog(view, request, window, cx);
                }
                ViewEvent::PrintRequested => self.offer_print(ToastAction::PrintPeek, cx),
                _ => {}
            }
        }
        cx.notify();
    }

    // ---- Boosts ------------------------------------------------------------------

    fn apply_boost(&self, id: TabId) {
        let (Some(live), Some(t)) = (self.live.get(&id), self.ws.tab(id)) else {
            return;
        };
        let Some(host) = host_of(&t.url) else { return };
        if let Some(boost) = self.boosts.for_host(&host)
            && boost.enabled
        {
            live.view.execute_js(&boosts::injection_script(boost));
        }
    }

    pub fn start_zap(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.active_tab() else { return };
        let Some(live) = self.live.get(&id) else { return };
        live.view.execute_js(boosts::ZAP_START_SCRIPT);
        self.zapping = Some(id);
        self.toast("Click elements to remove · Esc to finish", ToastStyle::Info, None, cx);
    }

    pub fn finish_zap(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.zapping.take() else { return };
        let (Some(live), Some(t)) = (self.live.get(&id), self.ws.tab(id)) else {
            return;
        };
        let Some(host) = host_of(&t.url) else { return };
        let weak = cx.entity().downgrade();
        let async_cx = cx.to_async();
        live.view.evaluate(boosts::ZAP_FINISH_SCRIPT, move |result| {
            let selectors: Vec<String> = result
                .ok()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            if selectors.is_empty() {
                return;
            }
            let _ = weak.update(&mut async_cx.clone(), |this, cx| {
                let mut boost = this
                    .boosts
                    .find_any(&host)
                    .cloned()
                    .unwrap_or_else(|| boosts::Boost::new(&host));
                for s in selectors {
                    if !boost.zapped_selectors.contains(&s) {
                        boost.zapped_selectors.push(s);
                    }
                }
                boost.enabled = true;
                let n = boost.zapped_selectors.len();
                this.boosts.upsert(boost);
                this.toast(format!("{n} element(s) hidden on {host}"), ToastStyle::Success, None, cx);
                this.schedule_save(cx);
            });
        });
    }

    pub fn open_boost_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.active_tab().and_then(|id| self.ws.tab(id)) else { return };
        let Some(host) = host_of(&t.url) else {
            self.toast("Boosts work on websites", ToastStyle::Info, None, cx);
            return;
        };
        let boost = self
            .boosts
            .find_any(&host)
            .cloned()
            .unwrap_or_else(|| boosts::Boost::new(&host));
        let editor = cx.new(|cx| crate::ui::boost_editor::BoostEditor::new(boost, window, cx));
        let sub = cx.subscribe_in(
            &editor,
            window,
            |this, _, ev: &crate::ui::boost_editor::BoostEditorEvent, window, cx| {
                use crate::ui::boost_editor::BoostEditorEvent as E;
                match ev {
                    E::Save(boost) => {
                        this.boosts.upsert(boost.clone());
                        if let Some(view) = this.selected_view() {
                            view.reload(false);
                        }
                        this.schedule_save(cx);
                    }
                    E::Remove(id) => {
                        this.boosts.remove(*id);
                        if let Some(view) = this.selected_view() {
                            view.reload(false);
                        }
                        this.schedule_save(cx);
                    }
                    E::Zap => {
                        this.boost_editor = None;
                        this.start_zap(cx);
                        this.focus_selected_page(window, cx);
                        return;
                    }
                    E::Cancel => {}
                }
                this.boost_editor = None;
                this.focus_selected_page(window, cx);
                cx.notify();
            },
        );
        self.boost_events = Some(sub);
        self.boost_editor = Some(editor);
        cx.notify();
    }

    // ---- Engine events -----------------------------------------------------------------

    fn on_view_events(
        &mut self,
        id: TabId,
        events: Vec<ViewEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut save = false;
        // Only events that change what the chrome shows redraw the window.
        let mut changed = false;
        for ev in events {
            changed |= !matches!(
                ev,
                ViewEvent::ConsoleMessage { .. }
                    | ViewEvent::Created
                    | ViewEvent::Painted
                    | ViewEvent::PopupWidgetChanged
                    | ViewEvent::CursorChanged(_)
                    | ViewEvent::Tooltip(_)
                    | ViewEvent::TextSelectionChanged(_)
                    | ViewEvent::ImeCompositionBounds(_)
                    | ViewEvent::LoadProgress(_)
                    | ViewEvent::AudioStateChanged { .. }
                    | ViewEvent::MediaAccessChanged { .. }
            );
            let Some(ev) = self.on_page_event(PageOwner::Tab(id), ev, window, cx) else { continue };
            match ev {
                ViewEvent::AddressChanged(url) => {
                    if let Some(t) = self.ws.tab_mut(id) {
                        if t.url != url {
                            t.scroll_y = 0.0;
                        }
                        t.url = url.clone();
                        save = true;
                    }
                    if let Some(live) = self.live.get_mut(&id) {
                        live.error = None;
                        live.reader = false;
                    }
                    self.apply_site_zoom(id);
                    if self
                        .typed_https
                        .get(&id)
                        .is_some_and(|typed| host_of(typed) != host_of(&url))
                    {
                        self.typed_https.remove(&id);
                    }
                }
                ViewEvent::TitleChanged(title) => {
                    if let Some(t) = self.ws.tab_mut(id) {
                        t.title = title.clone();
                        let url = t.url.split('#').next().unwrap_or(&t.url).to_string();
                        self.history.update_title(&url, &title);
                        save = true;
                    }
                }
                ViewEvent::FaviconUrlChanged(url) => {
                    if let Some(t) = self.ws.tab_mut(id) {
                        t.favicon_url = url;
                        save = true;
                    }
                }
                ViewEvent::Favicon(frame) => {
                    if let Some(host) = self.ws.tab(id).and_then(|t| host_of(&t.url)) {
                        self.favicons.insert(&host, frame, cx);
                    }
                }
                ViewEvent::LoadingStateChanged {
                    loading,
                    can_go_back,
                    can_go_forward,
                } => {
                    if let Some(live) = self.live.get_mut(&id) {
                        live.loading = loading;
                        live.can_go_back = can_go_back;
                        live.can_go_forward = can_go_forward;
                        if loading {
                            live.progress = live.progress.min(0.1);
                        }
                    }
                }
                ViewEvent::LoadProgress(p) => {
                    if let Some(live) = self.live.get_mut(&id) {
                        live.progress = p;
                    }
                }
                ViewEvent::LoadEnd { http_status } => {
                    if http_status > 0 {
                        self.typed_https.remove(&id);
                    }
                    if let Some(y) = self.pending_scroll.remove(&id)
                        && let Some(live) = self.live.get(&id)
                    {
                        live.view.execute_js(&format!("window.scrollTo(0, {y})"));
                    }
                    if let Some(t) = self.ws.tab_mut(id) {
                        t.last_accessed = mori_core::paths::now();
                        // In-page anchors are the same page for history.
                        let url = t.url.split('#').next().unwrap_or(&t.url).to_string();
                        let title = t.title.clone();
                        self.history.record(&url, &title);
                        save = true;
                    }
                    self.apply_boost(id);
                    if let Some(live) = self.live.get_mut(&id) {
                        live.blank_popup = false;
                        live.view.execute_js(mori_core::scripts::CLIPBOARD_BRIDGE_SCRIPT);
                        live.view.execute_js(mori_core::page_actions::PEEK_LINK_SCRIPT);
                    }
                    self.apply_site_zoom(id);
                    self.check_media_soon(id, cx);
                }
                ViewEvent::LoadError { code, text, url } => {
                    // A typed address Mori upgraded to https: retry over http.
                    let fallback = self
                        .typed_https
                        .get(&id)
                        .filter(|typed| host_of(typed) == host_of(&url))
                        .and_then(|_| navigation::http_fallback(&url, code));
                    if let Some(http) = fallback {
                        self.typed_https.remove(&id);
                        if let Some(t) = self.ws.tab_mut(id) {
                            t.url = http.clone();
                        }
                        if let Some(live) = self.live.get_mut(&id) {
                            live.error = None;
                            live.view.load_url(&http);
                        }
                        save = true;
                    } else if let Some(live) = self.live.get_mut(&id) {
                        live.error = Some(PageError { code, text, url });
                    }
                }
                ViewEvent::RenderProcessGone => {
                    if let Some(live) = self.live.get_mut(&id) {
                        live.error = Some(PageError {
                            code: 0,
                            text: "The page's renderer process stopped.".into(),
                            url: live.view.url(),
                        });
                    }
                }
                ViewEvent::NewView {
                    view,
                    url,
                    disposition,
                    features,
                    user_gesture,
                } => {
                    use mori_engine::Disposition as D;
                    // A window.open that asked for a size floats over its opener.
                    if popups::wants_window(disposition, features) {
                        self.open_popup_window(id, view, url, features, window, cx);
                        continue;
                    }
                    // Shift+click ("new window") previews the link in Peek.
                    if disposition == D::Window && user_gesture {
                        self.peek_view(view, &url, window, cx);
                        continue;
                    }
                    let background = disposition == D::BackgroundTab;
                    let mut record = TabRecord::new(url.clone());
                    record.title = url;
                    let last_child = self
                        .child_anchor
                        .filter(|(opener, _)| *opener == id)
                        .map(|(_, child)| child);
                    let new_id = self.ws.add_child_tab(record, id, last_child);
                    self.adopt_view(new_id, view, window, cx);
                    if let Some(live) = self.live.get_mut(&new_id) {
                        live.opener = Some(id);
                        live.blank_popup = true;
                    }
                    if background {
                        if let Some(live) = self.live.get(&new_id) {
                            live.view.set_hidden(true);
                        }
                        self.child_anchor = Some((id, new_id));
                    } else {
                        self.select_tab(new_id, window, cx);
                    }
                    save = true;
                }
                ViewEvent::CloseRequested => {
                    let asked = self.live.get(&id).is_some_and(|l| l.close_requested.is_some());
                    match self.live.get(&id).and_then(|l| l.opener) {
                        // A page that closes itself (OAuth popups) returns to its opener.
                        Some(opener) if !asked => self.close_tab_quietly(id, Some(opener), window, cx),
                        // The page agreed to the user's close (or called window.close()).
                        _ => self.remove_closed_tab(id, window, cx),
                    }
                }
                ViewEvent::FileDialog(request) => {
                    if let Some(view) = self.live.get(&id).map(|l| l.view.clone()) {
                        self.open_file_dialog(view, request, window, cx);
                    }
                }
                ViewEvent::PrintRequested => self.offer_print(ToastAction::PrintTab(id), cx),
                ViewEvent::FindResult { count, active, final_update } => {
                    // Intermediate updates report ordinal 0; keep the last real one.
                    if let Some(live) = self.live.get_mut(&id) {
                        live.find_count = count;
                        if active > 0 || count == 0 {
                            live.find_active = active;
                        }
                    }
                    if self.active_tab() == Some(id)
                        && let Some(bar) = &self.find_bar
                    {
                        bar.update(cx, |b, cx| b.set_results(count, active, final_update, cx));
                    }
                }
                ViewEvent::ContextMenu(info) => {
                    if let Some(origin) = self.card_origin(id, cx) {
                        let position =
                            origin + point(px(info.x as f32), px(info.y as f32));
                        self.menu = Some(OpenMenu {
                            kind: MenuKind::Web {
                                tab: id,
                                info: Box::new(info),
                            },
                            position,
                        });
                    }
                }
                ViewEvent::FullscreenChanged(on) => {
                    if let Some(live) = self.live.get_mut(&id) {
                        live.fullscreen = on;
                    }
                    if on != window.is_fullscreen() {
                        window.toggle_fullscreen();
                    }
                }
                ViewEvent::StatusMessage(text) => {
                    if let Some(live) = self.live.get_mut(&id) {
                        live.status = text;
                    }
                }
                ViewEvent::Closed => {
                    if self.live.get(&id).is_some_and(|l| l.view.is_closed()) {
                        self.live.remove(&id);
                    }
                }
                // Alt+click on a link in the visible page (PEEK_LINK_SCRIPT).
                ViewEvent::ConsoleMessage { message, .. } => {
                    // Any page can log the marker, so only act on it right
                    // after a real Alt+click; otherwise it would open
                    // previews without a user gesture (a popup-blocker bypass).
                    if self.visible_tabs().contains(&Some(id))
                        && let Some(url) = mori_core::page_actions::peek_request(&message)
                        && self.alt_click_at.take().is_some_and(|t| t.elapsed() < Duration::from_secs(2))
                    {
                        self.peek_url(&url, window, cx);
                    }
                }
                ViewEvent::Created
                | ViewEvent::Painted
                | ViewEvent::PopupWidgetChanged
                | ViewEvent::CursorChanged(_)
                | ViewEvent::TextSelectionChanged(_)
                | ViewEvent::ImeCompositionBounds(_)
                | ViewEvent::AudioStateChanged { .. }
                | ViewEvent::MediaAccessChanged { .. } => {}
                // Answered by `on_page_event`.
                ViewEvent::Tooltip(_)
                | ViewEvent::UnhandledKey(_)
                | ViewEvent::JsDialog(_)
                | ViewEvent::DialogsReset
                | ViewEvent::PermissionPrompt(_)
                | ViewEvent::PermissionDismissed(_)
                | ViewEvent::AuthRequired(_)
                | ViewEvent::CertificateError(_) => {}
            }
        }
        if save {
            self.schedule_save(cx);
        }
        if changed {
            cx.notify();
        }
    }

    /// Window-coordinate origin of a tab's page, as of its last paint.
    pub fn card_origin(&self, id: TabId, cx: &App) -> Option<Point<Pixels>> {
        let live = self.live.get(&id)?;
        Some(live.surface.read(cx).bounds().origin)
    }

    // ---- Page prompts: dialogs, credentials, certificates, permissions ---------------

    fn owner_view(&self, owner: PageOwner) -> Option<WebView> {
        match owner {
            PageOwner::Tab(id) => self.live.get(&id).map(|l| l.view.clone()),
            PageOwner::Peek => self.peek.as_ref().map(|p| p.view.clone()),
            PageOwner::Popup => self.popup.as_ref().map(|p| p.view.clone()),
        }
    }

    fn owner_surface(&self, owner: PageOwner) -> Option<Entity<WebSurface>> {
        match owner {
            PageOwner::Tab(id) => self.live.get(&id).map(|l| l.surface.clone()),
            PageOwner::Peek => self.peek.as_ref().map(|p| p.surface.clone()),
            PageOwner::Popup => self.popup.as_ref().map(|p| p.surface.clone()),
        }
    }

    /// Events every page handles the same way, tab or Peek. Returns the
    /// event when it is not one of them.
    pub(super) fn on_page_event(
        &mut self,
        owner: PageOwner,
        ev: ViewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ViewEvent> {
        match ev {
            ViewEvent::JsDialog(dialog) => self.open_dialog(owner, dialog, window, cx),
            ViewEvent::DialogsReset => {
                // Chromium also resets on navigations with no dialog open;
                // only hand focus back when a dialog actually went away, so
                // a navigating page doesn't take the keyboard from the chrome.
                if !self.dialogs.take_owner(owner).is_empty() {
                    self.sync_page_modals(cx);
                    self.refocus_owner(owner, window, cx);
                }
            }
            ViewEvent::PermissionPrompt(req) => self.on_permission_request(owner, req, cx),
            ViewEvent::PermissionDismissed(pid) => {
                self.permissions.take(|r| r.id == pid);
            }
            ViewEvent::AuthRequired(request) => self.open_auth(owner, request, window, cx),
            ViewEvent::CertificateError(err) => self.cert_errors.push(owner, err),
            ViewEvent::Tooltip(text) => {
                // Drawn by the surface itself; the chrome needn't redraw.
                if let Some(surface) = self.owner_surface(owner) {
                    surface.update(cx, |s, cx| s.set_tooltip(text, cx));
                }
                return None;
            }
            ViewEvent::UnhandledKey(key) => {
                if let Some(surface) = self.owner_surface(owner) {
                    surface.update(cx, |s, cx| s.run_unhandled_shortcut(key, window, cx));
                }
            }
            ev => return Some(ev),
        }
        cx.notify();
        None
    }

    /// Gray out (and stop input to) every page with an open dialog or
    /// credentials prompt.
    pub fn sync_page_modals(&mut self, cx: &mut Context<Self>) {
        let owners: Vec<PageOwner> = self
            .live
            .keys()
            .map(|t| PageOwner::Tab(*t))
            .chain(self.peek.as_ref().map(|_| PageOwner::Peek))
            .chain(self.popup.as_ref().map(|_| PageOwner::Popup))
            .collect();
        for owner in owners {
            let modal = self.dialogs.has(owner) || self.auth_prompts.has(owner);
            if let Some(surface) = self.owner_surface(owner) {
                surface.update(cx, |s, cx| s.set_inert(modal, cx));
            }
        }
    }

    /// What takes the keyboard while `owner` shows a modal prompt.
    fn modal_focus(&self, owner: PageOwner, cx: &App) -> Option<FocusHandle> {
        if let Some(d) = self.dialogs.front(owner) {
            return Some(match &d.input {
                Some(input) => input.read(cx).focus_handle(cx),
                None => self.dialog_focus.clone(),
            });
        }
        self.auth_prompts
            .front(owner)
            .map(|a| a.user.read(cx).focus_handle(cx))
    }

    /// Move focus to `owner`'s next prompt or back to the page, if `owner`
    /// is what the user is looking at.
    fn refocus_owner(&mut self, owner: PageOwner, window: &mut Window, cx: &mut Context<Self>) {
        match owner {
            PageOwner::Peek | PageOwner::Popup => {
                // The popup floats over the Peek: a Peek prompt under an
                // open popup waits for it.
                if owner == PageOwner::Peek && self.popup.is_some() {
                    return;
                }
                let target = self
                    .modal_focus(owner, cx)
                    .or_else(|| self.owner_surface(owner).map(|s| s.read(cx).focus_handle().clone()));
                if let Some(handle) = target {
                    window.focus(&handle, cx);
                }
            }
            PageOwner::Tab(id) if self.active_tab() == Some(id) && self.peek.is_none() && self.popup.is_none() => {
                self.focus_selected_page(window, cx);
            }
            PageOwner::Tab(_) => {}
        }
    }

    /// A Peek or floating popup covers the pages.
    pub fn page_overlay_open(&self) -> bool {
        self.peek.is_some() || self.popup.is_some()
    }

    /// The page whose prompts are on top: the popup or Peek if open, else
    /// the active tab (the focused split pane or the selected tab).
    pub fn front_owner(&self) -> Option<PageOwner> {
        if self.popup.is_some() {
            Some(PageOwner::Popup)
        } else if self.peek.is_some() {
            Some(PageOwner::Peek)
        } else {
            self.active_tab().map(PageOwner::Tab)
        }
    }

    /// A page changed hands (Peek or popup became a tab): its waiting
    /// prompts go with it.
    /// The connection state to show for `owner`'s page at `url`.
    pub fn page_security(&self, owner: PageOwner, url: &str) -> mori_core::security::Security {
        use mori_core::security;
        let error_page = match owner {
            PageOwner::Tab(id) => self
                .live
                .get(&id)
                .and_then(|l| l.error.as_ref())
                .is_some_and(|e| security::is_certificate_error_code(e.code)),
            _ => false,
        };
        // What Chromium itself recorded for the page on screen: this also
        // covers its own interstitial and a "Proceed" chosen there.
        let bad_cert = self
            .owner_view(owner)
            .and_then(|v| v.cert_status())
            .is_some_and(security::cert_status_is_error);
        security::classify(url, error_page || bad_cert || self.cert_errors.has(owner), &self.cert_exceptions)
    }

    pub(super) fn reassign_prompts(&mut self, from: PageOwner, to: PageOwner) {
        self.dialogs.reassign(from, to);
        self.auth_prompts.reassign(from, to);
        self.permissions.reassign(from, to);
        self.cert_errors.reassign(from, to);
    }

    pub(super) fn forget_prompts(&mut self, owner: PageOwner) {
        self.dialogs.take_owner(owner);
        self.auth_prompts.take_owner(owner);
        self.permissions.take_owner(owner);
        self.cert_errors.take_owner(owner);
    }

    fn cancel_navigation_prompts(&mut self, owner: PageOwner, cx: &mut Context<Self>) {
        let Some(view) = self.owner_view(owner) else { return };
        for auth in self.auth_prompts.take_owner(owner) {
            view.resolve_auth(auth.request.id, None);
        }
        for err in self.cert_errors.take_owner(owner) {
            view.resolve_certificate_error(err.id, false);
        }
        self.sync_page_modals(cx);
    }

    fn open_dialog(&mut self, owner: PageOwner, dialog: JsDialog, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::input::InputEvent;
        let id = dialog.id;
        let (input, subscription) = if dialog.kind == DialogKind::Prompt {
            let default = dialog.default_prompt.clone();
            let input = cx.new(|cx| {
                let mut s = InputState::new(window, cx);
                s.set_value(default, window, cx);
                s
            });
            let sub = cx.subscribe_in(&input, window, move |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.resolve_dialog(id, true, window, cx);
                }
            });
            (Some(input), Some(sub))
        } else {
            (None, None)
        };
        let leaving = dialog.kind == DialogKind::BeforeUnload;
        self.dialogs.push(owner, PendingDialog { dialog, input, _subscription: subscription });
        self.sync_page_modals(cx);
        // Dialogs from background tabs wait until the tab is shown, as in
        // Chrome; "Leave site?" for a tab being closed is shown right away.
        if let PageOwner::Tab(tab) = owner
            && leaving
            && self.selected() != Some(tab)
        {
            self.select_tab(tab, window, cx);
        }
        self.refocus_owner(owner, window, cx);
        cx.notify();
    }

    pub fn resolve_dialog(&mut self, id: mori_engine::DialogId, accept: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((owner, pending)) = self.dialogs.take(|d| d.dialog.id == id) else { return };
        let text = pending.input.as_ref().map(|i| i.read(cx).value().to_string());
        if pending.dialog.kind == DialogKind::BeforeUnload
            && !accept
            && let PageOwner::Tab(tab) = owner
            && let Some(live) = self.live.get_mut(&tab)
        {
            // "Stay": the tab keeps living and a later close asks again.
            live.close_requested = None;
        }
        if let Some(view) = self.owner_view(owner) {
            view.resolve_dialog(id, accept, text.as_deref());
        }
        self.sync_page_modals(cx);
        self.refocus_owner(owner, window, cx);
        cx.notify();
    }

    /// Escape: cancel the dialog or credentials prompt on top, if any.
    pub fn cancel_front_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(owner) = self.front_owner() else { return false };
        if let Some(id) = self.dialogs.front(owner).map(|d| d.dialog.id) {
            self.resolve_dialog(id, false, window, cx);
            return true;
        }
        if let Some(id) = self.auth_prompts.front(owner).map(|a| a.request.id) {
            self.resolve_auth(id, false, window, cx);
            return true;
        }
        false
    }

    fn on_permission_request(&mut self, owner: PageOwner, req: PermissionRequest, cx: &mut Context<Self>) {
        // A remembered answer for every kind asked skips the prompt.
        match remembered_permission(&self.site_permissions, &req) {
            Some(allow) => {
                if let Some(view) = self.owner_view(owner) {
                    view.resolve_permission(req.id, allow);
                }
            }
            None => self.permissions.push(owner, req),
        }
        cx.notify();
    }

    /// Answer a permission prompt and remember the answer for the site.
    pub fn resolve_permission(&mut self, id: mori_engine::PermissionId, allow: bool, cx: &mut Context<Self>) {
        let Some((owner, req)) = self.permissions.take(|r| r.id == id) else { return };
        let decision = if allow { Decision::Allow } else { Decision::Block };
        for key in req.kinds.iter().filter_map(|k| crate::ui::site_info::permission_key(*k)) {
            self.site_permissions.set(&req.origin, key, decision);
        }
        if let Some(view) = self.owner_view(owner) {
            view.resolve_permission(id, allow);
        }
        // Other waiting requests this answer now covers.
        let store = &self.site_permissions;
        while let Some((owner, other)) = self.permissions.take(|r| remembered_permission(store, r).is_some()) {
            let allow = remembered_permission(store, &other) == Some(true);
            if let Some(view) = self.owner_view(owner) {
                view.resolve_permission(other.id, allow);
            }
        }
        self.schedule_save(cx);
        cx.notify();
    }

    fn open_auth(&mut self, owner: PageOwner, request: AuthRequest, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::input::InputEvent;
        let id = request.id;
        let user = cx.new(|cx| InputState::new(window, cx).placeholder("Username"));
        let password = cx.new(|cx| InputState::new(window, cx).placeholder("Password").masked(true));
        let submit = move |this: &mut Self, _: &Entity<InputState>, ev: &InputEvent, window: &mut Window, cx: &mut Context<Self>| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.resolve_auth(id, true, window, cx);
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&user, window, submit),
            cx.subscribe_in(&password, window, submit),
        ];
        self.auth_prompts.push(owner, PendingAuth { request, user, password, _subscriptions: subscriptions });
        self.sync_page_modals(cx);
        self.refocus_owner(owner, window, cx);
        cx.notify();
    }

    /// Answer a credentials prompt: `submit` sends what was typed, otherwise
    /// the request is cancelled (and the server's 401 page shows).
    pub fn resolve_auth(&mut self, id: mori_engine::AuthId, submit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((owner, pending)) = self.auth_prompts.take(|a| a.request.id == id) else { return };
        if let Some(view) = self.owner_view(owner) {
            if submit {
                let user = pending.user.read(cx).value().to_string();
                let password = pending.password.read(cx).value().to_string();
                view.resolve_auth(id, Some((&user, &password)));
            } else {
                view.resolve_auth(id, None);
            }
        }
        self.sync_page_modals(cx);
        self.refocus_owner(owner, window, cx);
        cx.notify();
    }

    /// Answer a certificate error: proceed to the site anyway, or go back.
    pub fn resolve_certificate_error(&mut self, id: mori_engine::CertErrorId, proceed: bool, cx: &mut Context<Self>) {
        let Some((owner, err)) = self.cert_errors.take(|e| e.id == id) else { return };
        if proceed {
            // Chromium lets the host through for the rest of the session;
            // the address bar must keep saying it isn't secure.
            self.cert_exceptions.allow(&err.url);
        }
        if let Some(view) = self.owner_view(owner) {
            view.resolve_certificate_error(id, proceed);
            if !proceed && view.can_go_back() {
                view.go_back();
            }
        }
        cx.notify();
    }

    // ---- Maintenance: sleep & archive -----------------------------------------------------

    fn start_maintenance(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(60)).await;
                if this.update(cx, |this, cx| this.run_maintenance(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn run_maintenance(&mut self, cx: &mut Context<Self>) {
        self.last_maintenance = Instant::now();
        let now = mori_core::paths::now();
        // Tabs on screen or playing media are in use right now.
        let mut in_use: Vec<TabId> = self.visible_tabs().into_iter().flatten().collect();
        in_use.extend(self.media.iter().filter(|(_, m)| m.state.playing).map(|(id, _)| *id));
        for id in &in_use {
            if let Some(t) = self.ws.tab_mut(*id) {
                t.last_accessed = now;
            }
        }
        for id in self.visible_tabs().into_iter().flatten() {
            self.capture_scroll(id, cx);
        }
        let downloading: Vec<TabId> = self
            .downloads
            .iter()
            .filter(|d| d.state.is_active())
            .filter_map(|d| d.engine_id)
            .filter_map(|e| self.download_tabs.get(&DownloadId(e)).copied())
            .collect();
        let busy = |id: TabId| {
            let owner = PageOwner::Tab(id);
            in_use.contains(&id)
                || downloading.contains(&id)
                || self.dialogs.has(owner)
                || self.auth_prompts.has(owner)
                || self.permissions.has(owner)
                || self.cert_errors.has(owner)
        };
        let activity: Vec<TabActivity> = self
            .ws
            .tabs
            .iter()
            .map(|t| TabActivity {
                id: t.id,
                last_accessed: t.last_accessed,
                live: self.live.contains_key(&t.id),
                loose: self
                    .ws
                    .location(t.id)
                    .is_some_and(|l| matches!(l.slot, Slot::Loose(_))),
                web: t.url.starts_with("http://") || t.url.starts_with("https://"),
                busy: busy(t.id),
            })
            .collect();
        let archive = maintenance::archive_candidates(&activity, now, self.settings.auto_archive_hours);
        for id in &archive {
            self.unless_unsaved_input(*id, cx, |this, id, cx| this.archive_idle_tab(id, cx));
        }
        for id in maintenance::sleep_candidates(&activity, now, self.settings.auto_sleep_minutes) {
            if !archive.contains(&id) {
                self.unless_unsaved_input(id, cx, |this, id, cx| {
                    if !this.visible_tabs().contains(&Some(id)) {
                        this.unload(id, cx);
                    }
                });
            }
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Run `f` for a tab unless its page holds unsaved input. Sleeping
    /// pages have none, so `f` runs right away for them.
    fn unless_unsaved_input(
        &mut self,
        id: TabId,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, TabId, &mut Context<Self>) + 'static,
    ) {
        let Some(live) = self.live.get(&id) else {
            f(self, id, cx);
            return;
        };
        let weak = cx.entity().downgrade();
        let async_cx = cx.to_async();
        live.view.evaluate(maintenance::UNSAVED_INPUT_SCRIPT, move |result| {
            if matches!(result, Ok(serde_json::Value::Bool(false))) {
                let _ = weak.update(&mut async_cx.clone(), |this, cx| f(this, id, cx));
            }
        });
    }

    /// Auto-archive one idle tab: keep it in the archive and close it.
    fn archive_idle_tab(&mut self, id: TabId, cx: &mut Context<Self>) {
        if self.visible_tabs().contains(&Some(id)) {
            return;
        }
        let Some(t) = self.ws.tab(id) else { return };
        self.archive.add(&t.url, &t.title, t.favicon_url.clone());
        self.take_tab(id, false, cx);
        self.schedule_save(cx);
    }

    // ---- Window --------------------------------------------------------------------------

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.settings.sidebar_visible = !self.settings.sidebar_visible;
        self.sidebar_peek = false;
        self.sidebar_hide = None;
        self.schedule_save(cx);
        cx.notify();
    }

    /// The pointer moved to `x` in a window `width` wide (or left the window,
    /// `None`). With the sidebar hidden, touching the window edge reveals it
    /// as a floating card; it hides again once the pointer has stayed away
    /// from it for a moment, unless a menu or rename opened from it is up.
    pub fn track_sidebar_peek(&mut self, x: Option<f32>, width: f32, cx: &mut Context<Self>) {
        if self.settings.sidebar_visible {
            return;
        }
        let left = self.settings.sidebar_position == mori_core::settings::SidebarPosition::Left;
        let from_edge = x.map(|x| if left { x } else { width - x });
        let reach = self.settings.clamped_sidebar_width() + 8. + SIDEBAR_HIDE_MARGIN;
        match from_edge {
            Some(d) if d <= SIDEBAR_REVEAL_EDGE => {
                self.sidebar_hide = None;
                if !self.sidebar_peek {
                    self.sidebar_peek = true;
                    cx.notify();
                }
            }
            _ if !self.sidebar_peek => {}
            Some(d) if d <= reach => self.sidebar_hide = None,
            _ if self.menu.is_some() || self.rename.is_some() => self.sidebar_hide = None,
            _ => {
                if self.sidebar_hide.is_none() {
                    self.sidebar_hide = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(SIDEBAR_HIDE_DELAY).await;
                        this.update(cx, |this, cx| {
                            this.sidebar_hide = None;
                            if this.menu.is_none() && this.rename.is_none() {
                                this.sidebar_peek = false;
                                cx.notify();
                            }
                        })
                        .ok();
                    }));
                }
            }
        }
    }

    pub fn quit(&mut self, cx: &mut Context<Self>) {
        self.save_and_flush();
        cx.quit();
    }
}

/// The remembered answer to `req` (`Some(true)` allow, `Some(false)` block),
/// or `None` when the user must be asked: a kind Mori can't remember, or a
/// kind without an answer yet.
fn remembered_permission(
    store: &mori_core::site_permissions::SitePermissionStore,
    req: &PermissionRequest,
) -> Option<bool> {
    let keys: Option<Vec<&str>> = req
        .kinds
        .iter()
        .map(|k| crate::ui::site_info::permission_key(*k))
        .collect();
    keys.and_then(|keys| store.answer(&req.origin, &keys))
}
