//! Popover panels opened from the sidebar header: Downloads and Library
//! (history, bookmarks, archive).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Input, InputState},
    *,
};
use mori_core::downloads::{DownloadRecord, DownloadState};

use crate::{
    browser::{Browser, Panel},
    theme::{ActiveTokens, metrics},
    ui::components::{favicon, format_bytes, h_flex, icon, icon_button, popover_surface, relative_time, v_flex},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LibraryTab {
    #[default]
    History,
    Bookmarks,
    Archive,
}

impl Browser {
    pub fn render_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let panel = self.panel?;
        let body = match panel {
            Panel::Downloads => self.render_downloads(cx).into_any_element(),
            Panel::Library => self.render_library(window, cx).into_any_element(),
        };
        let left = self.settings.sidebar_position == mori_core::settings::SidebarPosition::Left;
        let offset = px(self.settings.clamped_sidebar_width() + 4.);
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.panel = None;
                        cx.notify();
                    }),
                )
                .child(
                    popover_surface(cx)
                        .absolute()
                        .top(px(44.))
                        .when(left, |s| s.left(offset))
                        .when(!left, |s| s.right(offset))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(body),
                )
                .into_any_element(),
        )
    }

    fn render_downloads(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let now = mori_core::paths::now();
        let rows: Vec<AnyElement> = self
            .downloads
            .iter()
            .enumerate()
            .map(|(i, d)| self.render_download_row(i, d, now, cx))
            .collect();
        let empty = rows.is_empty();
        let dir = self.downloads_dir.clone();
        v_flex()
            .w(px(360.))
            .max_h(px(420.))
            .child(
                h_flex()
                    .h(px(44.))
                    .px_3()
                    .gap_1()
                    .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child("Downloads"))
                    .child(
                        icon_button("dl-folder", "folder", px(26.), true, cx)
                            .on_click(move |_, _, _| open_path(&dir)),
                    )
                    .child(icon_button("dl-clear", "brush-cleaning", px(26.), true, cx).on_click(
                        cx.listener(|this, _, _, cx| this.clear_finished_downloads(cx)),
                    )),
            )
            .child(
                v_flex()
                    .id("downloads-list")
                    .p_1()
                    .overflow_y_scroll()
                    .when(empty, |s| {
                        s.child(
                            div()
                                .p_4()
                                .text_color(t.muted_foreground)
                                .child("No downloads yet"),
                        )
                    })
                    .children(rows),
            )
    }

    fn render_download_row(&self, i: usize, d: &DownloadRecord, now: f64, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let host = mori_core::util::host_of(&d.url).unwrap_or_default();
        let missing = d.state == DownloadState::Complete && !d.path.exists();
        let dot = |a: String, b: &str| if b.is_empty() { a } else { format!("{a} · {b}") };
        let of_total = || {
            if d.total > 0 {
                format!("{} of {}", format_bytes(d.received), format_bytes(d.total))
            } else {
                format_bytes(d.received)
            }
        };
        let status = match d.state {
            DownloadState::InProgress => format!("{} — {}/s", of_total(), format_bytes(d.speed)),
            DownloadState::Paused => format!("Paused — {}", of_total()),
            DownloadState::Complete if missing => dot("Deleted".into(), &host),
            DownloadState::Complete => {
                let when = d.finished_at.map(|f| relative_time(f, now)).unwrap_or_default();
                dot(dot(format_bytes(d.received.max(d.total)), &host), &when)
            }
            DownloadState::Canceled => dot("Canceled".into(), &host),
            DownloadState::Interrupted => dot("Failed".into(), &host),
        };
        let active = d.state.is_active();
        let fraction = d.fraction();
        let id = d.id;
        let path = d.path.clone();
        let file_icon = if matches!(d.state, DownloadState::Canceled | DownloadState::Interrupted) || missing {
            "file-x"
        } else if active {
            "file-down"
        } else {
            file_icon(&d.filename)
        };

        let mut actions = h_flex().gap_0p5().flex_none();
        match d.state {
            DownloadState::InProgress => {
                actions = actions
                    .child(icon_button(("dl-pause", i), "pause", px(24.), true, cx).on_click(
                        cx.listener(move |this, _, _, _| this.pause_download(id)),
                    ))
                    .child(icon_button(("dl-cancel", i), "x", px(24.), true, cx).on_click(
                        cx.listener(move |this, _, _, _| this.cancel_download(id)),
                    ));
            }
            DownloadState::Paused => {
                actions = actions
                    .child(icon_button(("dl-resume", i), "play", px(24.), true, cx).on_click(
                        cx.listener(move |this, _, _, _| this.resume_download(id)),
                    ))
                    .child(icon_button(("dl-cancel", i), "x", px(24.), true, cx).on_click(
                        cx.listener(move |this, _, _, _| this.cancel_download(id)),
                    ));
            }
            DownloadState::Complete if !missing => {
                let p = path.clone();
                actions = actions.child(
                    icon_button(("dl-reveal", i), "folder-open", px(24.), true, cx)
                        .on_click(move |_, _, _| reveal(&p)),
                );
            }
            DownloadState::Canceled | DownloadState::Interrupted => {
                actions = actions.child(icon_button(("dl-retry", i), "rotate-cw", px(24.), true, cx).on_click(
                    cx.listener(move |this, _, window, cx| this.retry_download(id, window, cx)),
                ));
            }
            DownloadState::Complete => {}
        }
        if !active {
            actions = actions.child(
                div()
                    .invisible()
                    .group_hover("download-row", |s| s.visible())
                    .child(icon_button(("dl-remove", i), "trash-2", px(24.), true, cx).on_click(
                        cx.listener(move |this, _, _, cx| this.remove_download(id, cx)),
                    )),
            );
        }

        h_flex()
            .id(("download", i))
            .group("download-row")
            .gap_3()
            .px_3()
            .py_2()
            .rounded(metrics::RADIUS_LG)
            .hover(|s| s.bg(t.ghost_hover()))
            .when(d.state == DownloadState::Complete && !missing, |s| s.cursor_pointer())
            .child(icon(file_icon, px(20.), t.muted_foreground))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .when(missing, |s| s.text_color(t.muted_foreground).line_through())
                            .child(d.filename.clone()),
                    )
                    .when(active, |s| {
                        s.child(
                            div()
                                .h(px(4.))
                                .rounded_full()
                                .overflow_hidden()
                                .bg(t.foreground.opacity(0.08))
                                .child(
                                    div()
                                        .h_full()
                                        .rounded_full()
                                        .bg(if d.state == DownloadState::Paused {
                                            t.muted_foreground
                                        } else {
                                            t.primary
                                        })
                                        .w(relative(fraction.unwrap_or(0.0))),
                                ),
                        )
                    })
                    .child(
                        div()
                            .truncate()
                            .text_size(metrics::TEXT_SMALL)
                            .text_color(t.muted_foreground)
                            .child(status),
                    ),
            )
            .child(actions)
            .when(d.state == DownloadState::Complete && !missing, |s| {
                s.on_click(move |_, _, _| open_path(&path))
            })
            .into_any_element()
    }

    fn library_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(input) = &self.library_search {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let sub = cx.observe(&input, |_, _, cx| cx.notify());
        self.push_subscription(sub);
        self.library_search = Some(input.clone());
        input
    }

    fn render_library(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let search = self.library_search(window, cx);
        let query = search.read(cx).value().to_string();
        let tab = self.library_tab;
        let now = mori_core::paths::now();

        let segment = |label: &'static str, value: LibraryTab, cx: &mut Context<Self>| {
            let active = tab == value;
            div()
                .id(label)
                .flex_1()
                .h(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(metrics::RADIUS_MD)
                .text_size(metrics::TEXT_LABEL)
                .cursor_pointer()
                .when(active, |s| s.bg(t.primary).text_color(t.primary_foreground))
                .when(!active, |s| s.hover(|s| s.bg(t.ghost_hover())))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.library_tab = value;
                    cx.notify();
                }))
        };

        struct Row {
            title: String,
            url: String,
            meta: String,
            remove: Option<RemoveKind>,
        }
        enum RemoveKind {
            History(uuid::Uuid),
            Bookmark(uuid::Uuid),
            Archive(uuid::Uuid),
        }
        let mut groups: Vec<(Option<String>, Vec<Row>)> = Vec::new();
        match tab {
            LibraryTab::History => {
                let entries: Vec<_> = if query.trim().is_empty() {
                    self.history.recent(200).iter().collect()
                } else {
                    self.history.search(&query, 200)
                };
                groups.push((
                    None,
                    entries
                        .into_iter()
                        .map(|h| Row {
                            title: h.display_title().to_string(),
                            url: h.url.clone(),
                            meta: relative_time(h.last_visited, now),
                            remove: Some(RemoveKind::History(h.id)),
                        })
                        .collect(),
                ));
            }
            LibraryTab::Bookmarks => groups.push((
                None,
                self.bookmarks
                    .search(&query)
                    .into_iter()
                    .map(|b| Row {
                        title: b.title.clone(),
                        url: b.url.clone(),
                        meta: String::new(),
                        remove: Some(RemoveKind::Bookmark(b.id)),
                    })
                    .collect(),
            )),
            LibraryTab::Archive => {
                let offset = local_utc_offset();
                for (group, entries) in self.archive.grouped(now, offset) {
                    let q = query.to_lowercase();
                    let rows: Vec<Row> = entries
                        .into_iter()
                        .filter(|e| {
                            q.is_empty()
                                || e.title.to_lowercase().contains(&q)
                                || e.url.to_lowercase().contains(&q)
                        })
                        .map(|e| Row {
                            title: e.title.clone(),
                            url: e.url.clone(),
                            meta: relative_time(e.archived_at, now),
                            remove: Some(RemoveKind::Archive(e.id)),
                        })
                        .collect();
                    if !rows.is_empty() {
                        groups.push((Some(group.label().to_string()), rows));
                    }
                }
            }
        }

        let mut list = v_flex().gap(px(1.));
        let mut index = 0usize;
        let mut any = false;
        for (label, rows) in groups {
            if let Some(label) = label {
                list = list.child(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_size(metrics::TEXT_SMALL)
                        .text_color(t.muted_foreground)
                        .child(label),
                );
            }
            for row in rows {
                any = true;
                let i = index;
                index += 1;
                let host = mori_core::util::host_of(&row.url);
                let fav = host.as_deref().and_then(|h| self.favicons.get(h));
                let url = row.url.clone();
                let remove = row.remove;
                let restore_archive = matches!(remove, Some(RemoveKind::Archive(_)));
                list = list.child(
                    h_flex()
                        .id(("library-row", i))
                        .group("library-row")
                        .h(px(30.))
                        .px_2()
                        .gap_2()
                        .rounded(metrics::RADIUS_LG)
                        .cursor_pointer()
                        .hover(|s| s.bg(t.ghost_hover()))
                        .child(favicon(fav, px(15.), false, cx))
                        .child(div().flex_1().min_w_0().truncate().child(if row.title.is_empty() {
                            row.url.clone()
                        } else {
                            row.title.clone()
                        }))
                        .child(
                            div()
                                .text_size(metrics::TEXT_SMALL)
                                .text_color(t.muted_foreground)
                                .child(row.meta),
                        )
                        .when_some(remove, |s, remove| {
                            s.child(
                                div()
                                    .id(("library-remove", i))
                                    .invisible()
                                    .group_hover("library-row", |s| s.visible())
                                    .child(icon("x", px(12.), t.muted_foreground))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        match &remove {
                                            RemoveKind::History(id) => {
                                                this.history.remove(*id);
                                            }
                                            RemoveKind::Bookmark(id) => {
                                                this.bookmarks.remove(*id);
                                            }
                                            RemoveKind::Archive(id) => {
                                                this.archive.remove(*id);
                                            }
                                        }
                                        this.schedule_save(cx);
                                        cx.notify();
                                    })),
                            )
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.panel = None;
                            if restore_archive {
                                let found = this
                                    .archive
                                    .entries
                                    .iter()
                                    .find(|e| e.url == url)
                                    .map(|e| e.id);
                                if let Some(id) = found {
                                    this.archive.remove(id);
                                    // Its "Archived … Undo" toast would bring
                                    // back a second copy.
                                    this.toasts.retain(|t| {
                                        !matches!(
                                            &t.action,
                                            Some((_, crate::browser::ToastAction::UndoArchive { entry, .. })) if *entry == id
                                        )
                                    });
                                }
                            }
                            this.open_url(&url, true, window, cx);
                        })),
                );
            }
        }
        if !any {
            list = list.child(div().p_4().text_color(t.muted_foreground).child("Nothing here yet"));
        }

        v_flex()
            .w(px(380.))
            .max_h(px(460.))
            .p_2()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .child(segment("History", LibraryTab::History, cx))
                    .child(segment("Bookmarks", LibraryTab::Bookmarks, cx))
                    .child(segment("Archive", LibraryTab::Archive, cx)),
            )
            .child(Input::new(&search).cleanable(true))
            .child(div().id("library-list").flex_1().overflow_y_scroll().child(list))
            .when(tab == LibraryTab::History, |s| {
                s.child(
                    div()
                        .id("clear-history")
                        .px_2()
                        .py_1()
                        .text_size(metrics::TEXT_LABEL)
                        .text_color(t.destructive)
                        .cursor_pointer()
                        .child("Clear browsing history")
                        .on_click(cx.listener(|this, _, _, cx| this.clear_history(cx))),
                )
            })
    }
}

/// Seconds east of UTC for grouping the archive by local day:
/// `MORI_UTC_OFFSET` if set, else the C library's local time zone. (This
/// runs on every render of the Archive tab, so it must not spawn anything.)
pub(crate) fn local_utc_offset() -> i64 {
    if let Some(offset) = std::env::var("MORI_UTC_OFFSET").ok().and_then(|v| v.parse().ok()) {
        return offset;
    }
    // SAFETY: `time` and `localtime_r` only write into the provided
    // out-parameters; `tm` is plain data.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            0
        } else {
            tm.tm_gmtoff
        }
    }
}

/// Lucide icon for a downloaded file, by extension.
fn file_icon(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "heic" | "bmp" | "ico" => "file-image",
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "rar" | "zst" | "deb" | "rpm" => "file-archive",
        "mp4" | "mkv" | "webm" | "mov" | "avi" => "file-video-camera",
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "opus" => "file-music",
        "pdf" | "txt" | "md" | "doc" | "docx" | "odt" | "rtf" | "epub" => "file-text",
        "csv" | "xls" | "xlsx" | "ods" => "file-spreadsheet",
        "json" | "js" | "ts" | "rs" | "py" | "html" | "css" | "xml" | "sh" | "toml" | "yaml" | "yml" => "file-code",
        _ => "file",
    }
}

pub fn open_path(path: &std::path::Path) {
    if let Ok(mut child) = std::process::Command::new("xdg-open").arg(path).spawn() {
        // Reap it, or every open leaves a zombie behind until Mori exits.
        std::thread::spawn(move || child.wait());
    }
}

/// Show a file in the file manager (its folder, with the file selected
/// where the file manager supports it).
pub fn reveal(path: &std::path::Path) {
    let path = path.to_path_buf();
    // Off the UI thread: the D-Bus call may wait on service activation.
    std::thread::spawn(move || {
        let uri = url::Url::from_file_path(&path).ok();
        let shown = uri.is_some_and(|uri| {
            std::process::Command::new("dbus-send")
                .args([
                    "--session",
                    "--print-reply",
                    "--type=method_call",
                    "--dest=org.freedesktop.FileManager1",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                ])
                .arg(format!("array:string:{uri}"))
                .arg("string:")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        });
        if !shown {
            open_path(path.parent().unwrap_or(&path));
        }
    });
}

/// The header's downloads button: a ring that fills with the progress of
/// running downloads.
pub fn download_ring(fraction: Option<f32>, active: bool, color: Hsla, track: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            if !active {
                return;
            }
            let center = bounds.center();
            let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2. - 1.5;
            let arc = |from: f32, to: f32| {
                let mut path = PathBuilder::stroke(px(2.));
                let steps = 48;
                for s in 0..=steps {
                    let a = from + (to - from) * s as f32 / steps as f32;
                    let p = point(
                        center.x + px(radius * a.sin()),
                        center.y - px(radius * a.cos()),
                    );
                    if s == 0 { path.move_to(p) } else { path.line_to(p) }
                }
                path.build().ok()
            };
            let tau = std::f32::consts::TAU;
            if let Some(track) = arc(0., tau).map(|p| (p, track)) {
                window.paint_path(track.0, track.1);
            }
            // Unknown size: a quarter arc marks activity.
            let (from, to) = match fraction {
                Some(f) => (0., tau * f.clamp(0.02, 1.)),
                None => (0., tau * 0.25),
            };
            if let Some(path) = arc(from, to) {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .inset_0()
}
