//! Popover panels opened from the sidebar header: Downloads and Library
//! (history, bookmarks, archive).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Input, InputState},
    *,
};
use mori_engine::DownloadState;

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
        let rows: Vec<AnyElement> = self
            .downloads
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let status = match d.state {
                    DownloadState::InProgress => {
                        let total = if d.total > 0 {
                            format!(" of {}", format_bytes(d.total))
                        } else {
                            String::new()
                        };
                        format!("{}{total} — {}/s", format_bytes(d.received), format_bytes(d.speed))
                    }
                    DownloadState::Paused => "Paused".into(),
                    DownloadState::Complete => format!("Completed — {}", format_bytes(d.received)),
                    DownloadState::Canceled => "Canceled".into(),
                    DownloadState::Interrupted => "Failed".into(),
                };
                let fraction = if d.percent >= 0 { d.percent as f32 / 100.0 } else { 0.3 };
                let in_progress = d.state == DownloadState::InProgress;
                let path = d.path.clone();
                let id = d.id;
                h_flex()
                    .id(("download", i))
                    .group("download-row")
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded(metrics::RADIUS_LG)
                    .hover(|s| s.bg(t.ghost_hover()))
                    .child(icon(
                        if d.state == DownloadState::Complete { "file-check" } else { "file-down" },
                        px(20.),
                        t.muted_foreground,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(div().truncate().font_weight(FontWeight::MEDIUM).child(d.filename.clone()))
                            .when(in_progress, |s| {
                                s.child(
                                    div()
                                        .h(px(4.))
                                        .rounded_full()
                                        .bg(t.foreground.opacity(0.08))
                                        .child(
                                            div()
                                                .h_full()
                                                .rounded_full()
                                                .bg(t.primary)
                                                .w(relative(fraction)),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .text_size(metrics::TEXT_SMALL)
                                    .text_color(t.muted_foreground)
                                    .child(status),
                            ),
                    )
                    .child(if in_progress {
                        icon_button(("dl-cancel", i), "x", px(24.), true, cx)
                            .on_click(move |_, _, _| mori_engine::cancel_download(id))
                            .into_any_element()
                    } else {
                        let p = path.clone();
                        icon_button(("dl-reveal", i), "folder-open", px(24.), true, cx)
                            .on_click(move |_, _, _| reveal(&p))
                            .into_any_element()
                    })
                    .on_click(move |ev: &ClickEvent, _, _| {
                        if ev.click_count() == 2 {
                            open_path(&path);
                        }
                    })
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        let dir = dirs::download_dir().unwrap_or_default();
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
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.history.clear();
                            this.schedule_save(cx);
                            this.toast("History cleared", crate::browser::ToastStyle::Success, None, cx);
                        })),
                )
            })
    }
}

/// Seconds east of UTC for grouping the archive by local day:
/// `MORI_UTC_OFFSET` if set, else what `date +%z` reports, else UTC.
fn local_utc_offset() -> i64 {
    std::env::var("MORI_UTC_OFFSET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| {
            std::process::Command::new("date")
                .arg("+%z")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| {
                    let s = s.trim();
                    let sign = if s.starts_with('-') { -1 } else { 1 };
                    let digits = s.trim_start_matches(['+', '-']);
                    let (h, m) = digits.split_at(2.min(digits.len()));
                    Some(sign * (h.parse::<i64>().ok()? * 3600 + m.parse::<i64>().unwrap_or(0) * 60))
                })
                .unwrap_or(0)
        })
}

pub fn open_path(path: &std::path::Path) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

fn reveal(path: &std::path::Path) {
    let dir = path.parent().unwrap_or(path);
    open_path(dir);
}
