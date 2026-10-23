//! Importing from other browsers: finding their profiles, running the
//! import off the UI thread with progress, and the import panel shown in
//! Settings > Import and in the first-run welcome.

use futures::StreamExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::import::{self, Options, SameSite, Source, Step, Summary};
use mori_engine::{BrowsingContext, privacy::CookieToSet};

use crate::{
    browser::{Browser, ToastStyle},
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon, text_button, v_flex},
};

/// Import state shared by Settings and the welcome.
#[derive(Default)]
pub struct ImportState {
    /// Profiles found on this computer; `None` until looked for.
    sources: Option<Vec<Source>>,
    detecting: bool,
    selected: usize,
    options: Option<Options>,
    running: Option<Running>,
    last: Option<Finished>,
}

struct Running {
    title: String,
    step: Step,
    /// Steps done and total.
    progress: (usize, usize),
    /// Cookies are in Chromium's jar, which answers after the read finishes.
    writing_cookies: bool,
}

struct Finished {
    title: String,
    summary: Summary,
    problems: Vec<String>,
}

impl ImportState {
    fn options(&self) -> Options {
        self.options.unwrap_or(Options::ALL)
    }

    fn source(&self) -> Option<&Source> {
        self.sources.as_ref()?.get(self.selected)
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// The last import added something.
    pub fn finished_with_changes(&self) -> bool {
        self.last.as_ref().is_some_and(|f| !f.summary.is_empty())
    }
}

/// "Google Chrome", or "Google Chrome — Work" when it has several profiles.
fn source_title(sources: &[Source], source: &Source) -> String {
    let several = sources.iter().filter(|s| s.kind == source.kind).count() > 1;
    source.title(several)
}

impl Browser {
    /// Look for importable browsers (once, in the background).
    pub fn detect_import_sources(&mut self, cx: &mut Context<Self>) {
        if self.import.sources.is_some() || self.import.detecting {
            return;
        }
        self.import.detecting = true;
        let found = cx.background_spawn(async move {
            dirs::home_dir().map(|home| import::detect(&home)).unwrap_or_default()
        });
        cx.spawn(async move |this, cx| {
            let sources = found.await;
            this.update(cx, |this, cx| {
                this.import.detecting = false;
                this.import.selected = import::suggested(&sources);
                this.import.sources = Some(sources);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Import the selected source with the chosen options.
    pub fn run_import(&mut self, cx: &mut Context<Self>) {
        if self.import.running.is_some() {
            return;
        }
        let Some(source) = self.import.source().cloned() else { return };
        let options = self.import.options().limited_to(source.contents);
        if !options.any() {
            return;
        }
        let sources = self.import.sources.clone().unwrap_or_default();
        let title = source_title(&sources, &source);
        self.import.last = None;
        self.import.running = Some(Running {
            title: title.clone(),
            step: Step::Bookmarks,
            progress: (0, 1),
            writing_cookies: false,
        });
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<(Step, usize, usize)>();
        let reader = source.clone();
        let read = cx.background_spawn(async move {
            import::read(&reader, options, &mut |step, i, n| {
                let _ = tx.unbounded_send((step, i, n));
            })
        });
        cx.spawn(async move |this, cx| {
            while let Some((step, i, n)) = rx.next().await {
                let alive = this.update(cx, |this, cx| {
                    if let Some(r) = this.import.running.as_mut() {
                        r.step = step;
                        r.progress = (i, n);
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let imported = read.await;
            this.update(cx, |this, cx| {
                let mut summary = import::apply(
                    source.kind,
                    &imported,
                    &mut this.ws,
                    &mut this.bookmarks,
                    &mut this.history,
                );
                let batch = cookies_to_set(&imported.cookies);
                let mut problems = imported.problems;
                if batch.is_empty() {
                    if !imported.cookies.is_empty() {
                        problems.push("Couldn't save cookies".into());
                    }
                    this.finish_import(title, summary, problems, cx);
                    return;
                }
                if let Some(r) = this.import.running.as_mut() {
                    r.writing_cookies = true;
                    cx.notify();
                }
                let context = cookie_context(this);
                let (tx, rx) = futures::channel::oneshot::channel();
                mori_engine::privacy::set_cookies(&context, batch, move |n| {
                    let _ = tx.send(n);
                });
                cx.spawn(async move |this, cx| {
                    let n = rx.await.unwrap_or(0);
                    this.update(cx, |this, cx| {
                        summary.cookies = n;
                        if n == 0 {
                            problems.push("Couldn't save cookies".into());
                        }
                        this.finish_import(title, summary, problems, cx);
                    })
                    .ok();
                })
                .detach();
            })
            .ok();
        })
        .detach();
    }

    fn finish_import(&mut self, title: String, summary: Summary, problems: Vec<String>, cx: &mut Context<Self>) {
        self.import.running = None;
        let text = format!("{title}: {}", summary.describe());
        let style = if problems.is_empty() { ToastStyle::Success } else { ToastStyle::Warning };
        self.toast(text, style, None, cx);
        self.import.last = Some(Finished { title, summary, problems });
        self.schedule_save(cx);
        cx.notify();
    }

    /// The source list, options, Import button and progress or result.
    pub fn render_import_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.detect_import_sources(cx);
        let t = cx.tokens().clone();
        let Some(sources) = self.import.sources.clone() else {
            return div()
                .py(px(12.))
                .text_color(t.muted_foreground)
                .child("Looking for other browsers…")
                .into_any_element();
        };
        if sources.is_empty() {
            return v_flex()
                .gap(px(6.))
                .child(div().font_weight(FontWeight::MEDIUM).child("No other browsers found"))
                .child(note(
                    format!(
                        "Mori imports from Chrome, Chromium, Brave, Vivaldi, Edge, Firefox and Zen. For Arc, copy \
                         {} from your Mac into your Downloads folder.",
                        import::ARC_SIDEBAR_FILE
                    ),
                    cx,
                ))
                .into_any_element();
        }
        let running = self.import.running.is_some();
        let selected = self.import.selected.min(sources.len() - 1);
        let options = self.import.options();
        let contents = sources[selected].contents;

        let rows: Vec<AnyElement> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let active = i == selected;
                h_flex()
                    .id(("import-source", i))
                    .min_h(px(44.))
                    .px(px(10.))
                    .gap(px(10.))
                    .rounded(metrics::RADIUS_LG)
                    .border_1()
                    .border_color(if active { t.primary.opacity(0.8) } else { t.border })
                    .when(active, |r| r.bg(t.primary.opacity(0.08)))
                    .when(!active && !running, |r| r.hover(|r| r.bg(t.ghost_hover())))
                    .when(!running, |r| r.cursor_pointer())
                    .child(radio(active, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().font_weight(FontWeight::MEDIUM).child(source_title(&sources, s)))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(metrics::TEXT_SMALL)
                                    .text_color(t.muted_foreground)
                                    .child(s.contents.describe()),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.import.running.is_none() {
                            this.import.selected = i;
                            cx.notify();
                        }
                    }))
                    .into_any_element()
            })
            .collect();

        let checkbox = |id: &'static str, label: &'static str, on: bool, available: bool, set: fn(&mut Options, bool), cx: &mut Context<Self>| {
            let checked = on && available;
            h_flex()
                .id(id)
                .gap(px(8.))
                .when(!available, |c| c.opacity(0.45))
                .when(available && !running, |c| c.cursor_pointer())
                .child(
                    div()
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(if checked { t.primary } else { crate::ui::components::control_stroke(&t) })
                        .when(checked, |b| b.bg(t.primary))
                        .when(checked, |b| b.child(icon("check", px(12.), t.primary_foreground))),
                )
                .child(div().text_size(metrics::TEXT_LABEL).child(label))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if available && this.import.running.is_none() {
                        let mut o = this.import.options();
                        set(&mut o, !on);
                        this.import.options = Some(o);
                        cx.notify();
                    }
                }))
        };
        let can_import = !running && options.limited_to(contents).any();

        let status: Option<AnyElement> = if let Some(r) = &self.import.running {
            let fraction = (r.progress.0 as f32 + 0.5) / r.progress.1.max(1) as f32;
            Some(
                v_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(metrics::TEXT_LABEL)
                            .text_color(t.muted_foreground)
                            .child(format!(
                                "{} — {}",
                                r.title,
                                if r.writing_cookies { "Saving cookies…" } else { r.step.label() }
                            )),
                    )
                    .child(
                        div()
                            .h(px(4.))
                            .w_full()
                            .rounded_full()
                            .bg(t.foreground.opacity(0.08))
                            .child(div().h_full().w(relative(fraction.clamp(0.05, 1.))).rounded_full().bg(t.primary)),
                    )
                    .into_any_element(),
            )
        } else {
            self.import.last.as_ref().map(|f| {
                v_flex()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .text_size(metrics::TEXT_LABEL)
                            .child(icon("circle-check", px(14.), t.primary))
                            .child(format!("{}: {}", f.title, f.summary.describe())),
                    )
                    .children(f.problems.iter().map(|p| {
                        div()
                            .text_size(metrics::TEXT_SMALL)
                            .text_color(t.status_warning)
                            .child(p.clone())
                    }))
                    .into_any_element()
            })
        };

        v_flex()
            .gap(px(12.))
            .child(v_flex().gap(px(6.)).children(rows))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(16.))
                    .child(checkbox("import-bookmarks", "Bookmarks", options.bookmarks, contents.bookmarks, |o, v| o.bookmarks = v, cx))
                    .child(checkbox("import-history", "History", options.history, contents.history, |o, v| o.history = v, cx))
                    .child(checkbox("import-cookies", "Cookies", options.cookies, contents.cookies, |o, v| o.cookies = v, cx))
                    .child(checkbox("import-tabs", "Open tabs & spaces", options.tabs, contents.tabs, |o, v| o.tabs = v, cx)),
            )
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(
                        text_button("import-run", if running { "Importing…" } else { "Import" }, true, cx)
                            .flex_none()
                            .when(!can_import, |b| b.opacity(0.5).cursor_default())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if can_import {
                                    this.run_import(cx);
                                }
                            })),
                    )
                    .child(div().flex_1().min_w_0().children(status)),
            )
            .into_any_element()
    }
}

/// Cookies go into the profile this window browses with. A private window
/// keeps nothing, so they land in the default profile instead.
fn cookie_context(browser: &Browser) -> BrowsingContext {
    if browser.kind == crate::windows::WindowKind::Private {
        BrowsingContext::shared()
    } else {
        crate::windows::context_for_space(browser, browser.ws.active_space)
    }
}

fn cookies_to_set(cookies: &[mori_core::import::ImportedCookie]) -> Vec<CookieToSet> {
    cookies
        .iter()
        .filter_map(|cookie| {
            let (url, domain) = cookie.storage_target()?;
            Some(CookieToSet {
                url,
                domain,
                name: cookie.name.clone(),
                value: cookie.value.clone(),
                path: cookie.path.clone(),
                expires_unix: cookie.expires,
                secure: cookie.secure,
                http_only: cookie.http_only,
                same_site: match cookie.same_site {
                    SameSite::Unspecified => mori_engine::privacy::SameSiteMode::Unspecified,
                    SameSite::None => mori_engine::privacy::SameSiteMode::None,
                    SameSite::Lax => mori_engine::privacy::SameSiteMode::Lax,
                    SameSite::Strict => mori_engine::privacy::SameSiteMode::Strict,
                },
            })
        })
        .collect()
}

fn radio(on: bool, cx: &App) -> Div {
    let t = cx.tokens();
    div()
        .size(px(16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(if on { t.primary } else { crate::ui::components::control_stroke(t) })
        .when(on, |r| r.child(div().size(px(8.)).rounded_full().bg(t.primary)))
}

fn note(text: impl Into<SharedString>, cx: &App) -> Div {
    let t = cx.tokens();
    div()
        .text_size(metrics::TEXT_SMALL)
        .text_color(t.muted_foreground)
        .child(text.into())
}
