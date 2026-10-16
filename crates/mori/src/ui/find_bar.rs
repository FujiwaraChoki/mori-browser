//! Find-in-page bar, pinned to the top-right of the web card.

use gpui_kit::{
    component::input::{Escape, Input, InputEvent, InputState},
    *,
};

use crate::{
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon, icon_button, popover_surface},
};

pub enum FindBarEvent {
    Search { text: String, forward: bool, next: bool },
    Close,
}

pub struct FindBar {
    input: Entity<InputState>,
    count: i32,
    active: i32,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FindBarEvent> for FindBar {}

impl FindBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find on page"));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, _, cx| match ev {
            InputEvent::Change => {
                let text = this.text(cx);
                cx.emit(FindBarEvent::Search {
                    text,
                    forward: true,
                    next: false,
                });
            }
            InputEvent::PressEnter { shift, .. } => this.step(!*shift, cx),
            _ => {}
        });
        FindBar {
            input,
            count: 0,
            active: 0,
            _subscriptions: vec![sub],
        }
    }

    fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
    }

    pub fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if !text.is_empty() {
            cx.emit(FindBarEvent::Search {
                text,
                forward,
                next: true,
            });
        }
    }

    pub fn set_results(&mut self, count: i32, active: i32, cx: &mut Context<Self>) {
        self.count = count;
        self.active = active;
        cx.notify();
    }

    pub fn reset_results(&mut self, cx: &mut Context<Self>) {
        self.set_results(0, 0, cx);
    }
}

impl Render for FindBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let has_text = !self.text(cx).is_empty();
        let label = if !has_text {
            String::new()
        } else if self.count == 0 {
            "No results".to_string()
        } else {
            format!("{}/{}", self.active, self.count)
        };
        popover_surface(cx)
            .absolute()
            .top(px(10.))
            .right(px(14.))
            .occlude()
            .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                cx.emit(FindBarEvent::Close);
                cx.stop_propagation();
            }))
            .child(
                h_flex()
                    .px(px(10.))
                    .py(px(6.))
                    .gap_2()
                    .child(icon("search", px(14.), t.muted_foreground))
                    .child(div().w(px(180.)).child(
                        Input::new(&self.input)
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(false),
                    ))
                    .child(
                        div()
                            .min_w(px(56.))
                            .text_size(metrics::TEXT_LABEL)
                            .text_color(t.muted_foreground)
                            .child(label),
                    )
                    .child(div().w(px(1.)).h(px(18.)).bg(t.border))
                    .child(
                        icon_button("find-prev", "chevron-up", px(24.), true, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.step(false, cx))),
                    )
                    .child(
                        icon_button("find-next", "chevron-down", px(24.), true, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.step(true, cx))),
                    )
                    .child(
                        icon_button("find-close", "x", px(24.), true, cx)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(FindBarEvent::Close))),
                    ),
            )
    }
}
