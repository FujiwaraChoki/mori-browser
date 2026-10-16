//! Page-level services the chrome provides to Chromium: which pages are
//! visible, per-site zoom, file choosers and printing.

use std::path::PathBuf;

use gpui_kit::*;
use mori_core::{util::host_of, workspace::TabId, zoom};
use mori_engine::{FileDialogMode, FileDialogRequest, WebView};

use super::{Browser, ToastAction, ToastStyle};

/// Test hook: when set, file choosers do not open the system picker but
/// answer with these `:`-separated paths (empty = the user cancelled).
/// Headless test runs use it because the portal picker would open on the
/// tester's real desktop.
const TEST_FILE_CHOOSER_ENV: &str = "MORI_TEST_FILE_CHOOSER";

impl Browser {
    /// Tell Chromium which pages are on screen. Hidden pages stop painting
    /// and get throttled timers: background tabs, the page behind Settings,
    /// and every page while the window is minimized or on another
    /// workspace. Called on each render and when window visibility changes.
    pub fn sync_view_visibility(&self, _cx: &App) {
        let selected = self.selected();
        let pages = self.window_visible && !self.settings_open;
        let fullscreen = self.selected_live().is_some_and(|l| l.fullscreen);
        for (tab, live) in &self.live {
            let shown = Some(*tab) == selected || (Some(*tab) == self.split && !fullscreen);
            live.view.set_hidden(!(pages && shown));
        }
        for (tab, pane) in &self.devtools {
            pane.view.set_hidden(!(pages && !fullscreen && Some(*tab) == selected));
        }
        if let Some(peek) = &self.peek {
            peek.view.set_hidden(!self.window_visible);
        }
        if let Some(popup) = &self.popup {
            popup.view.set_hidden(!self.window_visible);
        }
    }

    // ---- Zoom ------------------------------------------------------------------

    /// Zoom the selected page one Chrome step in (`direction > 0`), out
    /// (`< 0`), or back to 100% (`0`). Zoom is per site: every tab on the
    /// host follows, and it is remembered.
    pub fn zoom(&mut self, direction: f64, cx: &mut Context<Self>) {
        let Some(id) = self.selected() else { return };
        let Some(live) = self.live.get(&id) else { return };
        let step = if direction > 0.0 { 1 } else if direction < 0.0 { -1 } else { 0 };
        let level = zoom::step(live.view.zoom_level(), step);
        live.view.set_zoom_level(level);
        if let Some(host) = self.ws.tab(id).and_then(|t| host_of(&t.url)) {
            self.zoom.set(&host, level);
            for (tab, other) in &self.live {
                if *tab != id && self.ws.tab(*tab).and_then(|t| host_of(&t.url)).as_deref() == Some(host.as_str()) {
                    other.view.set_zoom_level(level);
                }
            }
            self.schedule_save(cx);
        }
        let action = (!zoom::is_default(level)).then(|| ("Reset".into(), ToastAction::ResetZoom));
        self.toast_keyed(
            "zoom",
            format!("Zoom {}%", zoom::percent_for_level(level)),
            ToastStyle::Info,
            action,
            cx,
        );
        cx.notify();
    }

    /// Zoom level of the selected page's site, for the address bar.
    pub fn selected_zoom_level(&self) -> f64 {
        self.selected()
            .and_then(|id| self.ws.tab(id))
            .and_then(|t| host_of(&t.url))
            .map(|host| self.zoom.level(&host))
            .unwrap_or(0.0)
    }

    /// Give a tab its site's remembered zoom after it navigated.
    pub(super) fn apply_site_zoom(&self, id: TabId) {
        let (Some(live), Some(tab)) = (self.live.get(&id), self.ws.tab(id)) else { return };
        let Some(host) = host_of(&tab.url) else { return };
        let level = self.zoom.level(&host);
        if (live.view.zoom_level() - level).abs() > 0.001 {
            live.view.set_zoom_level(level);
        }
    }

    // ---- File choosers -----------------------------------------------------------

    /// Answer a page's file chooser with the system picker (the desktop
    /// portal, attached to the Mori window).
    pub(super) fn open_file_dialog(
        &mut self,
        view: WebView,
        request: FileDialogRequest,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Ok(answer) = std::env::var(TEST_FILE_CHOOSER_ENV) {
            let paths: Vec<PathBuf> = answer.split(':').filter(|p| !p.is_empty()).map(PathBuf::from).collect();
            view.resolve_file_dialog(request.id, Some(paths));
            return;
        }
        let id = request.id;
        let receiver = match request.mode {
            FileDialogMode::Save => {
                let suggested = PathBuf::from(&request.default_path);
                let dir = suggested
                    .parent()
                    .filter(|p| p.is_dir())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.downloads_dir.clone());
                let name = suggested.file_name().map(|n| n.to_string_lossy().into_owned());
                let rx = cx.prompt_for_new_path(&dir, name.as_deref());
                cx.spawn(async move |_, _| {
                    let path = rx.await.ok().and_then(|r| r.ok()).flatten();
                    view.resolve_file_dialog(id, path.map(|p| vec![p]));
                })
                .detach();
                return;
            }
            mode => cx.prompt_for_paths(PathPromptOptions {
                files: mode != FileDialogMode::OpenFolder,
                directories: mode == FileDialogMode::OpenFolder,
                multiple: mode == FileDialogMode::OpenMultiple,
                prompt: None,
            }),
        };
        cx.spawn(async move |this, cx| {
            let result = receiver.await.ok();
            let paths = match result {
                Some(Ok(paths)) => paths,
                Some(Err(err)) => {
                    log::warn!("file chooser failed: {err:#}");
                    this.update(cx, |this, cx| {
                        this.toast("Couldn't open the file chooser", ToastStyle::Error, None, cx);
                    })
                    .ok();
                    None
                }
                None => None,
            };
            view.resolve_file_dialog(id, paths);
        })
        .detach();
    }

    // ---- Printing ------------------------------------------------------------------

    /// Print the selected page (⌘P / the Print command).
    pub fn print_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected() {
            self.print_tab(id, cx);
        }
    }

    pub(super) fn print_tab(&mut self, id: TabId, cx: &mut Context<Self>) {
        let Some(view) = self.live.get(&id).map(|l| l.view.clone()) else { return };
        let title = self.ws.tab(id).map(|t| t.display_title().to_string()).unwrap_or_default();
        self.print_view(view, title, cx);
    }

    /// Offscreen pages cannot show Chromium's print dialog, so printing
    /// saves a PDF to the downloads folder and opens it in the system
    /// viewer, which prints it.
    pub(super) fn print_view(&mut self, view: WebView, title: String, cx: &mut Context<Self>) {
        let name = format!("{}.pdf", pdf_name(&title));
        let _ = std::fs::create_dir_all(&self.downloads_dir);
        let path = mori_engine::unique_path(&self.downloads_dir, &name);
        let url = view.url();
        self.toast_keyed("print", "Preparing PDF…", ToastStyle::Info, None, cx);
        let weak = cx.entity().downgrade();
        let async_cx = cx.to_async();
        let target = path.clone();
        view.print_to_pdf(&path, move |ok| {
            let _ = weak.update(&mut async_cx.clone(), |this, cx| {
                if !ok {
                    this.toast_keyed("print", "Couldn't print this page", ToastStyle::Error, None, cx);
                    return;
                }
                let size = std::fs::metadata(&target).map(|m| m.len() as i64).unwrap_or(0);
                this.downloads.add_file(&url, &target, size, mori_core::paths::now());
                this.schedule_save(cx);
                crate::ui::panels::open_path(&target);
                let file = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                this.toast_keyed(
                    "print",
                    format!("Saved {file}"),
                    ToastStyle::Success,
                    Some(("Show in Folder".into(), ToastAction::Reveal(target.clone()))),
                    cx,
                );
            });
        });
    }
}

/// A file name for a page's PDF.
fn pdf_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned: String = cleaned.chars().take(80).collect();
    if cleaned.is_empty() { "Page".into() } else { cleaned }
}

#[cfg(test)]
mod tests {
    use super::pdf_name;

    #[test]
    fn pdf_names_are_safe() {
        assert_eq!(pdf_name("A/B: c?"), "A B c");
        assert_eq!(pdf_name("   "), "Page");
        assert_eq!(pdf_name(&"x".repeat(200)).len(), 80);
    }
}
