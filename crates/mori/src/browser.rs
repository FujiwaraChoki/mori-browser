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
    omnibox::{self, Resolved},
    persist::Saver,
    prompts::{Decision, PromptQueue, SitePermissions},
    routes::RouteStore,
    settings::Settings,
    util::host_of,
    workspace::{ClosedTab, FolderId, Slot, SpaceId, TabId, TabRecord, Workspace},
    zoom::ZoomStore,
};
use mori_engine::{
    AuthRequest, CertificateError, ContextMenuInfo, DialogKind, JsDialog, PermissionKind,
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

pub const NEW_TAB_URL: &str = "mori://newtab";

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
}

/// A right-click menu currently on screen.
#[derive(Clone, Debug)]
pub enum MenuKind {
    Web { tab: TabId, info: Box<ContextMenuInfo> },
    Tab(TabId),
    Folder(FolderId),
    Space(SpaceId),
    Sidebar,
    NewMenu,
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
}

pub struct Browser {
    pub ws: Workspace,
    pub settings: Settings,
    pub history: HistoryStore,
    pub bookmarks: BookmarkStore,
    pub archive: ArchiveStore,
    pub routes: RouteStore,
    pub boosts: BoostStore,
    pub favicons: Favicons,

    pub live: HashMap<TabId, LiveTab>,
    pub closed_stack: Vec<ClosedTab>,
    pub downloads: DownloadStore,
    pub downloads_dir: PathBuf,
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
    find_events: Option<Subscription>,
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
    /// Allow/Block answers given this session.
    site_permissions: SitePermissions<PermissionKind>,
    pub rename: Option<(RenameTarget, Entity<gpui_kit::component::input::InputState>)>,
    rename_events: Option<Subscription>,
    pub boost_editor: Option<Entity<crate::ui::boost_editor::BoostEditor>>,
    boost_events: Option<Subscription>,
    pub zapping: Option<TabId>,
    pub sidebar_peek: bool,
    pub sidebar_drag: Option<f32>,
    pub multi_select: Vec<TabId>,
    /// The assistant panel, kept alive while hidden so its conversation and
    /// Codex server survive toggling.
    pub assistant: Option<Entity<crate::ui::assistant::AssistantPanel>>,
    pub assistant_visible: bool,
    /// A second tab shown beside the selected one.
    pub split: Option<TabId>,
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
    save_scheduled: bool,
    last_maintenance: Instant,
    _subscriptions: Vec<Subscription>,
}

impl Browser {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let data_dir = mori_core::paths::data_dir();
        let settings = Settings::load(&data_dir);
        let mut ws = Workspace::load(&data_dir);
        if let Ok(url) = std::env::var("MORI_START_URL") {
            ws = Workspace::new();
            ws.add_tab(TabRecord::new(url), None, Slot::Loose(0));
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
            favicons: Favicons::new(data_dir.join("favicons")),
            ws,
            settings,
            live: HashMap::new(),
            closed_stack: Vec::new(),
            downloads,
            downloads_dir: mori_core::paths::downloads_dir(),
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
            find_events: None,
            menu: None,
            panel: None,
            settings_open: false,
            peek: None,
            dialogs: PromptQueue::default(),
            dialog_focus: cx.focus_handle(),
            auth_prompts: PromptQueue::default(),
            permissions: PromptQueue::default(),
            cert_errors: PromptQueue::default(),
            site_permissions: SitePermissions::default(),
            rename: None,
            rename_events: None,
            boost_editor: None,
            boost_events: None,
            zapping: None,
            sidebar_peek: false,
            sidebar_drag: None,
            multi_select: Vec::new(),
            assistant: None,
            assistant_visible: false,
            split: None,
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
            save_scheduled: false,
            last_maintenance: Instant::now(),
            _subscriptions: Vec::new(),
        };
        this.settings.sidebar_visible = this.settings.show_sidebar_on_launch;

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
            this.new_tab_with_url(NEW_TAB_URL, window, cx);
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
        use mori_core::{archive, bookmarks, boosts, downloads, history, routes, settings, workspace, zoom};
        let saver = &mut self.saver;
        let results = [
            ("session", saver.stage(workspace::SESSION_FILE, &self.ws)),
            ("settings", saver.stage(settings::SETTINGS_FILE, &self.settings)),
            ("history", saver.stage(history::HISTORY_FILE, &self.history)),
            ("bookmarks", saver.stage(bookmarks::BOOKMARKS_FILE, &self.bookmarks)),
            ("archive", saver.stage(archive::ARCHIVE_FILE, &self.archive)),
            ("routes", saver.stage(routes::ROUTES_FILE, &self.routes)),
            ("boosts", saver.stage(boosts::BOOSTS_FILE, &self.boosts)),
            ("downloads", saver.stage(downloads::DOWNLOADS_FILE, &self.downloads)),
            ("zoom", saver.stage(zoom::ZOOM_FILE, &self.zoom)),
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
        let ttl = if action.is_some() {
            Duration::from_secs(5)
        } else {
            Duration::from_millis(2400)
        };
        self.toasts.push(Toast {
            id,
            text: text.into(),
            style,
            action,
        });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ttl).await;
            this.update(cx, |this, cx| {
                this.toasts.retain(|t| t.id != id);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn dismiss_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        self.toasts.retain(|t| t.id != id);
        cx.notify();
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
                let tab = self.ws.restore_tab(*closed);
                self.closed_stack.pop();
                self.select_tab(tab, window, cx);
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

    pub fn selected_live(&self) -> Option<&LiveTab> {
        self.selected().and_then(|id| self.live.get(&id))
    }

    pub fn selected_view(&self) -> Option<WebView> {
        self.selected_live().map(|l| l.view.clone())
    }

    pub fn is_internal(url: &str) -> bool {
        url.is_empty() || url.starts_with("mori://") || url == "about:blank"
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

    /// Drop a tab's Chromium page but keep the tab (sleep).
    pub fn unload(&mut self, id: TabId, cx: &mut Context<Self>) {
        if self.split == Some(id) {
            self.split = None;
        }
        if let Some(live) = self.live.remove(&id) {
            EngineHub::unregister(cx, live.view.id());
            live.view.close();
        }
        self.drop_devtools(id, cx);
        self.drop_popup_of(id, cx);
        // Closing the view already answered its callbacks.
        self.forget_prompts(PageOwner::Tab(id));
        cx.notify();
    }

    pub fn select_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let previous_space = self.ws.active_space;
        let previous_selected = self.selected();
        if !self.ws.select(id) {
            return;
        }
        if previous_space != self.ws.active_space {
            self.apply_theme(Some(window), cx);
        }
        self.settings_open = false;
        self.multi_select.clear();
        if self.split == Some(id) {
            self.split = previous_selected.filter(|p| *p != id);
        }
        if self.split.is_some_and(|s| self.ws.space_of(s) != Some(self.ws.active_space)) {
            self.split = None;
        }
        self.realize(id, window, cx);
        let split = self.split;
        for (tab, live) in &self.live {
            let hidden = *tab != id && Some(*tab) != split;
            live.view.set_hidden(hidden);
            if hidden {
                live.surface.update(cx, |s, _| s.blur());
            }
        }
        if let Some(find) = &self.find_bar {
            find.update(cx, |f, cx| f.reset_results(cx));
        }
        self.focus_selected_page(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    pub fn focus_selected_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launcher.is_some() || self.rename.is_some() {
            return;
        }
        // A page's open dialog takes the keyboard instead of the page.
        if let Some(owner) = self.selected().map(PageOwner::Tab)
            && let Some(handle) = self.modal_focus(owner, cx)
        {
            window.focus(&handle, cx);
            return;
        }
        match self.selected_live() {
            Some(live) => {
                let handle = live.surface.read(cx).focus_handle().clone();
                window.focus(&handle, cx);
            }
            None => window.focus(&self.focus_handle, cx),
        }
    }

    pub fn new_tab_with_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) -> TabId {
        let record = TabRecord::new(url);
        let id = self.ws.add_tab_after(record, self.selected().filter(|s| !self.ws.is_pinned(*s)));
        self.select_tab(id, window, cx);
        id
    }

    /// Open user input (URL or search) in a new tab or the current one.
    pub fn open_input(&mut self, input: &str, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        let url = match omnibox::resolve(input, &self.settings) {
            Resolved::Url(u) | Resolved::Search(u) => u,
        };
        self.open_url(&url, new_tab, window, cx);
    }

    pub fn open_url(&mut self, url: &str, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        // An empty "New Tab" page is reused rather than left behind.
        let selected_is_blank = self
            .selected()
            .and_then(|id| self.ws.tab(id))
            .is_some_and(|t| Self::is_internal(&t.url) && !self.settings_open);
        if new_tab && selected_is_blank {
            if let Some(id) = self.selected() {
                self.navigate(id, url, window, cx);
            }
        } else if new_tab || self.selected().is_none() {
            self.new_tab_with_url(url, window, cx);
        } else if let Some(id) = self.selected() {
            self.navigate(id, url, window, cx);
        }
    }

    pub fn navigate(&mut self, id: TabId, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.ws.tab_mut(id) {
            t.url = url.to_string();
        }
        // A credentials or certificate question about the old page is moot.
        self.cancel_navigation_prompts(PageOwner::Tab(id), cx);
        if let Some(live) = self.live.get_mut(&id) {
            live.error = None;
            live.reader = false;
            if Self::is_internal(url) {
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
            let neighbor = self.ws.neighbor(id);
            self.unload(id, cx);
            if self.selected() == Some(id)
                && let Some(n) = neighbor
            {
                self.select_tab(n, window, cx);
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
        self.unload(id, cx);
        self.multi_select.retain(|t| *t != id);
        let Some(closed) = self.ws.remove_tab(id) else { return };
        let title: String = closed.record.display_title().chars().take(40).collect();
        self.closed_stack.push(closed.clone());
        if self.closed_stack.len() > 25 {
            self.closed_stack.remove(0);
        }
        self.toast(
            format!("Closed {title}"),
            ToastStyle::Info,
            Some(("Undo".into(), ToastAction::UndoClose(Box::new(closed)))),
            cx,
        );
        // Always keep at least one tab in the space.
        if self.ws.active().ordered().is_empty() {
            self.new_tab_with_url(NEW_TAB_URL, window, cx);
        } else if was_selected {
            match self.ws.selected() {
                Some(next) => self.select_tab(next, window, cx),
                None => {
                    if let Some(first) = self.ws.active().visible().first().copied() {
                        self.select_tab(first, window, cx);
                    }
                }
            }
        }
        self.schedule_save(cx);
        cx.notify();
    }

    pub fn reopen_closed_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(closed) = self.closed_stack.pop() {
            let id = self.ws.restore_tab(closed);
            self.select_tab(id, window, cx);
        }
    }

    pub fn duplicate_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.ws.tab(id).cloned() else { return };
        let mut copy = TabRecord::new(record.url);
        copy.title = record.title;
        copy.favicon_url = record.favicon_url;
        let new_id = self.ws.add_tab_after(copy, Some(id));
        self.select_tab(new_id, window, cx);
    }

    pub fn close_other_tabs(&mut self, keep: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let others: Vec<TabId> = self
            .ws
            .active()
            .loose
            .iter()
            .copied()
            .filter(|t| *t != keep)
            .collect();
        for id in others {
            self.close_tab(id, true, window, cx);
        }
    }

    pub fn close_tabs_below(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let loose = self.ws.active().loose.clone();
        if let Some(i) = loose.iter().position(|t| *t == id) {
            for &t in &loose[i + 1..] {
                self.close_tab(t, true, window, cx);
            }
        }
    }

    pub fn sleep_tab(&mut self, id: TabId, cx: &mut Context<Self>) {
        if self.selected() != Some(id) {
            self.unload(id, cx);
        }
    }

    pub fn sleep_background_tabs(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected();
        let ids: Vec<TabId> = self.live.keys().copied().filter(|t| Some(*t) != selected).collect();
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

    pub fn archive_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if self.ws.is_pinned(id) {
            return;
        }
        if let Some(t) = self.ws.tab(id) {
            self.archive.add(&t.url, &t.title, t.favicon_url.clone());
        }
        self.close_tab(id, true, window, cx);
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
        if let Some(id) = self.ws.cycle(delta) {
            self.select_tab(id, window, cx);
        }
    }

    pub fn select_ordinal(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.ws.by_ordinal(n) {
            self.select_tab(id, window, cx);
        }
    }

    // ---- Split view ------------------------------------------------------------

    /// Show `tab` beside the selected tab.
    pub fn open_split(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if Some(tab) == self.selected() || self.ws.space_of(tab) != Some(self.ws.active_space) {
            return;
        }
        self.split = Some(tab);
        self.realize(tab, window, cx);
        if let Some(live) = self.live.get(&tab) {
            live.view.set_hidden(false);
        }
        cx.notify();
    }

    pub fn close_split(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.split.take() {
            if let Some(live) = self.live.get(&tab) {
                live.view.set_hidden(true);
                live.surface.update(cx, |s, _| s.blur());
            }
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
                self.new_tab_with_url(NEW_TAB_URL, window, cx);
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

    pub fn delete_space(&mut self, space: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.ws.space(space).map(|s| s.ordered()).unwrap_or_default();
        for id in &ids {
            self.unload(*id, cx);
        }
        match self.ws.delete_space(space) {
            Some(closed) => {
                self.routes.remove_for_context(space);
                self.toast(
                    format!("Deleted space and {} tabs", closed.len()),
                    ToastStyle::Info,
                    None,
                    cx,
                );
                let active = self.ws.active_space;
                self.switch_space(active, window, cx);
            }
            None => self.toast("Can't delete the last space", ToastStyle::Warning, None, cx),
        }
        self.schedule_save(cx);
    }

    /// Move a tab to another space. If it was the selected tab, a neighbour
    /// in the current space takes over (or a fresh new tab if it was alone).
    pub fn move_tab_to_space(&mut self, tab: TabId, space: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        let was_selected = self.selected() == Some(tab);
        self.ws.move_to_space(tab, space);
        if was_selected {
            match self.ws.selected().or_else(|| self.ws.active().visible().first().copied()) {
                Some(next) => self.select_tab(next, window, cx),
                None => {
                    self.new_tab_with_url(NEW_TAB_URL, window, cx);
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
                .selected()
                .and_then(|id| self.ws.tab(id))
                .map(|t| omnibox::display_url(&t.url))
                .unwrap_or_default(),
        };
        let candidates = crate::ui::launcher::candidates(self, mode);
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
                        self.new_tab_with_url(NEW_TAB_URL, window, cx);
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
                // One subscription per bar: re-opening must not add another.
                let sub = cx.subscribe_in(&bar, window, |this, _, ev: &FindBarEvent, window, cx| {
                    let Some(view) = this.selected_view() else { return };
                    match ev {
                        FindBarEvent::Search { text, forward, next } => {
                            if text.is_empty() {
                                view.stop_finding(true);
                            } else {
                                view.find(text, *forward, false, *next);
                            }
                        }
                        FindBarEvent::Close => this.close_find(window, cx),
                    }
                });
                self.find_events = Some(sub);
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

    pub fn copy_url(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.selected().and_then(|id| self.ws.tab(id))
            && !Self::is_internal(&t.url)
        {
            cx.write_to_clipboard(ClipboardItem::new_string(t.url.clone()));
            self.toast("Copied URL", ToastStyle::Success, None, cx);
        }
    }

    pub fn toggle_bookmark(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.selected().and_then(|id| self.ws.tab(id)).cloned() else {
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
        let Some(id) = self.selected() else { return };
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
        if let Some(id) = self.selected() {
            self.navigate(id, &home, window, cx);
        }
    }

    pub fn peek_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(old) = self.peek.take() {
            EngineHub::unregister(cx, old.view.id());
            old.view.close();
            // The old page's questions must not land on the new one.
            self.forget_prompts(PageOwner::Peek);
            self.sync_page_modals(cx);
        }
        let (w, h) = self.card_size(window);
        let size = ((w as f32 * 0.74).min(1000.) as i32, (h as f32 * 0.82).min(820.) as i32 - 46);
        let view = WebView::new(url, size, window.scale_factor());
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

    /// Turn the peeked page into a real tab in the active space.
    pub fn promote_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek) = self.peek.take() else { return };
        EngineHub::unregister(cx, peek.view.id());
        let mut record = TabRecord::new(peek.view.url());
        record.title = peek.title;
        let id = self.ws.add_tab_after(record, self.selected());
        self.ws.select(id);
        self.dialogs.reassign(PageOwner::Peek, PageOwner::Tab(id));
        self.auth_prompts.reassign(PageOwner::Peek, PageOwner::Tab(id));
        self.permissions.reassign(PageOwner::Peek, PageOwner::Tab(id));
        self.cert_errors.reassign(PageOwner::Peek, PageOwner::Tab(id));
        self.adopt_view(id, peek.view, window, cx);
        self.select_tab(id, window, cx);
        self.sync_page_modals(cx);
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
                ViewEvent::LoadEnd { .. } => {
                    peek.view.execute_js(mori_core::scripts::CLIPBOARD_BRIDGE_SCRIPT)
                }
                ViewEvent::AddressChanged(u) => peek.url = u,
                ViewEvent::NewView { view, url, disposition, .. } => {
                    use mori_engine::Disposition as D;
                    // Keep the new page alive as a tab so window.opener keeps
                    // working. A popup (sign-in flows) needs its opener too,
                    // so the peeked page becomes a tab first.
                    match disposition {
                        D::Popup | D::Window => self.promote_peek(window, cx),
                        D::BackgroundTab => {}
                        _ => self.close_peek(window, cx),
                    }
                    let mut record = TabRecord::new(url.clone());
                    record.title = url;
                    let id = self.ws.add_tab_after(record, self.selected());
                    self.adopt_view(id, view, window, cx);
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
        let Some(id) = self.selected() else { return };
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
        let Some(t) = self.selected().and_then(|id| self.ws.tab(id)) else { return };
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
                        t.url = url.clone();
                        save = true;
                    }
                    if let Some(live) = self.live.get_mut(&id) {
                        live.error = None;
                        live.reader = false;
                    }
                    self.apply_site_zoom(id);
                    self.route_tab(id, &url, window, cx);
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
                ViewEvent::LoadEnd { .. } => {
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
                    }
                    self.apply_site_zoom(id);
                    self.check_media_soon(id, cx);
                }
                ViewEvent::LoadError { code, text, url } => {
                    if let Some(live) = self.live.get_mut(&id) {
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
                    ..
                } => {
                    use mori_engine::Disposition as D;
                    if popups::wants_window(disposition, features) {
                        self.open_popup_window(id, view, url, features, window, cx);
                        continue;
                    }
                    let background = disposition == D::BackgroundTab;
                    let mut record = TabRecord::new(url.clone());
                    record.title = url;
                    let new_id = self.ws.add_tab_after(record, Some(id));
                    self.adopt_view(new_id, view, window, cx);
                    if let Some(live) = self.live.get_mut(&new_id) {
                        live.opener = Some(id);
                        live.blank_popup = true;
                    }
                    if background {
                        if let Some(live) = self.live.get(&new_id) {
                            live.view.set_hidden(true);
                        }
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
                ViewEvent::FindResult { count, active, .. } => {
                    // Intermediate updates report ordinal 0; keep the last real one.
                    if let Some(live) = self.live.get_mut(&id) {
                        live.find_count = count;
                        if active > 0 || count == 0 {
                            live.find_active = active;
                        }
                        let active = live.find_active;
                        if let Some(bar) = &self.find_bar {
                            bar.update(cx, |b, cx| b.set_results(count, active, cx));
                        }
                    }
                }
                ViewEvent::ContextMenu(info) => {
                    if let Some(origin) = self.card_origin(cx) {
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
                ViewEvent::ConsoleMessage { .. }
                | ViewEvent::Created
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

    /// Air Traffic Control: move a tab into the space a routing rule names.
    fn route_tab(&mut self, id: TabId, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = host_of(url) else { return };
        let Some(route) = self.routes.match_host(&host) else { return };
        let target = route.context_id;
        let Some(loc) = self.ws.location(id) else { return };
        if loc.space == target || loc.slot.is_pinned() || self.ws.space(target).is_none() {
            return;
        }
        let was_selected = self.selected() == Some(id);
        self.ws.move_to_space(id, target);
        let name = self.ws.space(target).map(|s| s.name.clone()).unwrap_or_default();
        if was_selected {
            self.select_tab(id, window, cx);
        }
        self.toast(format!("Routed to {name}"), ToastStyle::Info, None, cx);
    }

    /// Window-coordinate origin of the selected page, as of its last paint.
    pub fn card_origin(&self, cx: &App) -> Option<Point<Pixels>> {
        let id = self.selected()?;
        let live = self.live.get(&id)?;
        Some(live.surface.read(cx).bounds().origin)
    }

    // ---- Page prompts: dialogs, credentials, certificates, permissions ---------------

    fn owner_view(&self, owner: PageOwner) -> Option<WebView> {
        match owner {
            PageOwner::Tab(id) => self.live.get(&id).map(|l| l.view.clone()),
            PageOwner::Peek => self.peek.as_ref().map(|p| p.view.clone()),
        }
    }

    fn owner_surface(&self, owner: PageOwner) -> Option<Entity<WebSurface>> {
        match owner {
            PageOwner::Tab(id) => self.live.get(&id).map(|l| l.surface.clone()),
            PageOwner::Peek => self.peek.as_ref().map(|p| p.surface.clone()),
        }
    }

    /// Events every page handles the same way, tab or Peek. Returns the
    /// event when it is not one of them.
    fn on_page_event(
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
            PageOwner::Peek => {
                let target = self
                    .modal_focus(owner, cx)
                    .or_else(|| self.peek.as_ref().map(|p| p.surface.read(cx).focus_handle().clone()));
                if let Some(handle) = target {
                    window.focus(&handle, cx);
                }
            }
            PageOwner::Tab(id) if self.selected() == Some(id) && self.peek.is_none() => {
                self.focus_selected_page(window, cx);
            }
            PageOwner::Tab(_) => {}
        }
    }

    /// The page whose prompts are on top: the Peek if open, else the
    /// selected tab.
    pub fn front_owner(&self) -> Option<PageOwner> {
        if self.peek.is_some() {
            Some(PageOwner::Peek)
        } else {
            self.selected().map(PageOwner::Tab)
        }
    }

    fn forget_prompts(&mut self, owner: PageOwner) {
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
        match self.site_permissions.decide(&req.origin, &req.kinds) {
            Some(decision) => {
                if let Some(view) = self.owner_view(owner) {
                    view.resolve_permission(req.id, decision == Decision::Allow);
                }
            }
            None => self.permissions.push(owner, req),
        }
        cx.notify();
    }

    pub fn resolve_permission(&mut self, id: mori_engine::PermissionId, allow: bool, cx: &mut Context<Self>) {
        let Some((owner, req)) = self.permissions.take(|r| r.id == id) else { return };
        let decision = if allow { Decision::Allow } else { Decision::Block };
        self.site_permissions.record(&req.origin, &req.kinds, decision);
        if let Some(view) = self.owner_view(owner) {
            view.resolve_permission(id, allow);
        }
        // Other requests from the same site this answer covers.
        while let Some((owner, other)) = self.permissions.take(|r| {
            self.site_permissions.decide(&r.origin, &r.kinds).is_some()
        }) {
            if let Some(view) = self.owner_view(owner) {
                let allow = self.site_permissions.decide(&other.origin, &other.kinds) == Some(Decision::Allow);
                view.resolve_permission(other.id, allow);
            }
        }
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
        let Some((owner, _)) = self.cert_errors.take(|e| e.id == id) else { return };
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
        let selected = self.selected();
        // Archive: idle, unpinned, loose http(s) tabs, really closed.
        if self.settings.auto_archive_hours > 0 {
            let cutoff = now - self.settings.auto_archive_hours as f64 * 3600.0;
            let doomed: Vec<TabId> = self
                .ws
                .spaces
                .iter()
                .flat_map(|s| s.loose.iter().copied())
                .filter(|id| Some(*id) != selected)
                .filter(|id| {
                    self.ws.tab(*id).is_some_and(|t| {
                        t.last_accessed < cutoff
                            && (t.url.starts_with("http://") || t.url.starts_with("https://"))
                    })
                })
                .collect();
            for id in doomed {
                if let Some(t) = self.ws.tab(id) {
                    self.archive.add(&t.url, &t.title, t.favicon_url.clone());
                }
                self.unload(id, cx);
                self.ws.remove_tab(id);
            }
        }
        // Sleep: idle background pages lose their renderer.
        if self.settings.auto_sleep_minutes > 0 {
            let cutoff = now - self.settings.auto_sleep_minutes as f64 * 60.0;
            let idle: Vec<TabId> = self
                .live
                .keys()
                .copied()
                .filter(|id| Some(*id) != selected)
                .filter(|id| self.ws.tab(*id).is_some_and(|t| t.last_accessed < cutoff))
                .collect();
            for id in idle {
                self.unload(id, cx);
            }
        }
        self.schedule_save(cx);
        cx.notify();
    }

    // ---- Window --------------------------------------------------------------------------

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.settings.sidebar_visible = !self.settings.sidebar_visible;
        self.sidebar_peek = false;
        self.schedule_save(cx);
        cx.notify();
    }

    pub fn quit(&mut self, cx: &mut Context<Self>) {
        self.save_and_flush();
        cx.quit();
    }
}
