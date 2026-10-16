//! Boost editor: per-site custom CSS/JS and zapped elements.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Escape, Textarea, TextareaState},
    *,
};
use mori_core::boosts::Boost;

use crate::{
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, popover_surface, text_button, v_flex},
};

pub enum BoostEditorEvent {
    Save(Boost),
    Remove(uuid::Uuid),
    Zap,
    Cancel,
}

pub struct BoostEditor {
    boost: Boost,
    css: Entity<TextareaState>,
    js: Entity<TextareaState>,
}

impl EventEmitter<BoostEditorEvent> for BoostEditor {}

impl BoostEditor {
    pub fn new(boost: Boost, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let make = |value: &str, window: &mut Window, cx: &mut Context<Self>| {
            let value = value.to_string();
            cx.new(|cx| {
                let mut s = TextareaState::new(window, cx);
                s.set_value(value, window, cx);
                s
            })
        };
        let css = make(&boost.css, window, cx);
        let js = make(&boost.js, window, cx);
        css.update(cx, |s, cx| s.focus(window, cx));
        BoostEditor { boost, css, js }
    }

    fn current(&self, cx: &App) -> Boost {
        let mut b = self.boost.clone();
        b.css = self.css.read(cx).value().to_string();
        b.js = self.js.read(cx).value().to_string();
        b
    }
}

impl Render for BoostEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let zapped = self.boost.zapped_selectors.len();
        let enabled = self.boost.enabled;
        let label = |s: &str| {
            div()
                .text_size(metrics::TEXT_SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted_foreground)
                .child(s.to_uppercase())
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(black().opacity(0.28))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(BoostEditorEvent::Cancel)))
            .child(
                popover_surface(cx)
                    .w(px(460.))
                    .shadow(t.modal_shadow())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                        cx.emit(BoostEditorEvent::Cancel);
                        cx.stop_propagation();
                    }))
                    .child(
                        h_flex()
                            .h(px(56.))
                            .px_4()
                            .gap_2()
                            .border_b_1()
                            .border_color(t.border.opacity(0.6))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .child(div().font_weight(FontWeight::SEMIBOLD).text_size(metrics::TEXT_TITLE).child("Boost"))
                                    .child(
                                        div()
                                            .text_size(metrics::TEXT_SMALL)
                                            .text_color(t.muted_foreground)
                                            .child(self.boost.host.clone()),
                                    ),
                            )
                            .child(
                                text_button("boost-enabled", if enabled { "Enabled" } else { "Disabled" }, enabled, cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.boost.enabled = !this.boost.enabled;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .p_4()
                            .gap_3()
                            .child(label("Zapped elements"))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(text_button("boost-zap", "Zap an Element", false, cx).on_click(
                                        cx.listener(|_, _, _, cx| cx.emit(BoostEditorEvent::Zap)),
                                    ))
                                    .child(div().flex_1().text_color(t.muted_foreground).child(format!("{zapped} hidden")))
                                    .when(zapped > 0, |s| {
                                        s.child(text_button("boost-clear", "Clear", false, cx).on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.boost.zapped_selectors.clear();
                                                cx.notify();
                                            }),
                                        ))
                                    }),
                            )
                            .child(label("Custom CSS"))
                            .child(
                                div()
                                    .h(px(140.))
                                    .font_family("monospace")
                                    .text_size(metrics::TEXT_LABEL)
                                    .child(Textarea::new(&self.css).h_full()),
                            )
                            .child(label("Custom JavaScript"))
                            .child(
                                div()
                                    .h(px(140.))
                                    .font_family("monospace")
                                    .text_size(metrics::TEXT_LABEL)
                                    .child(Textarea::new(&self.js).h_full()),
                            )
                            .child(
                                div()
                                    .text_size(metrics::TEXT_SMALL)
                                    .text_color(t.status_warning)
                                    .child("Custom JavaScript runs with this site's full privileges."),
                            ),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .pb_4()
                            .gap_2()
                            .child(text_button("boost-remove", "Remove Boost", false, cx).on_click(
                                cx.listener(|this, _, _, cx| cx.emit(BoostEditorEvent::Remove(this.boost.id))),
                            ))
                            .child(div().flex_1())
                            .child(text_button("boost-cancel", "Cancel", false, cx).on_click(
                                cx.listener(|_, _, _, cx| cx.emit(BoostEditorEvent::Cancel)),
                            ))
                            .child(text_button("boost-save", "Save", true, cx).on_click(cx.listener(
                                |this, _, _, cx| {
                                    let boost = this.current(cx);
                                    cx.emit(BoostEditorEvent::Save(boost))
                                },
                            ))),
                    ),
            )
    }
}
