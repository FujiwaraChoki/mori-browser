//! Assistant side panel. The conversation backend lives in
//! `crate::assistant`; this file only renders it and collects input.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
    *,
};

use crate::{
    assistant::{AssistantSession, Message, Role},
    browser::Browser,
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon, icon_button, text_button, v_flex},
};

pub struct AssistantPanel {
    browser: WeakEntity<Browser>,
    input: Entity<InputState>,
    session: Entity<AssistantSession>,
    _subscriptions: Vec<Subscription>,
}

impl AssistantPanel {
    pub fn new(browser: WeakEntity<Browser>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Ask anything…"));
        let session = cx.new(|cx| AssistantSession::new(browser.clone(), cx));
        let s1 = cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary: false, shift: false } = ev {
                let text = input.read(cx).value().to_string();
                input.update(cx, |s, cx| s.set_value("", window, cx));
                this.send(text, window, cx);
            }
        });
        let s2 = cx.observe(&session, |_, _, cx| cx.notify());
        AssistantPanel {
            browser,
            input,
            session,
            _subscriptions: vec![s1, s2],
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.focus(window, cx));
    }

    pub fn send(&mut self, text: String, _window: &mut Window, cx: &mut Context<Self>) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.session.update(cx, |s, cx| s.send(text, cx));
    }

    fn render_message(&self, i: usize, m: &Message, cx: &App) -> AnyElement {
        let t = cx.tokens();
        match m.role {
            Role::User => h_flex()
                .justify_end()
                .child(
                    div()
                        .max_w(relative(0.85))
                        .px_3()
                        .py_2()
                        .rounded(metrics::RADIUS_POPOVER)
                        .bg(t.primary.opacity(0.16))
                        .child(m.text.clone()),
                )
                .into_any_element(),
            Role::Assistant => div()
                .id(("assistant-msg", i))
                .text_color(t.foreground.opacity(0.92))
                .child(m.text.clone())
                .into_any_element(),
            Role::Tool => h_flex()
                .gap_2()
                .text_size(metrics::TEXT_SMALL)
                .text_color(t.muted_foreground)
                .child(icon("wrench", px(12.), t.muted_foreground))
                .child(m.text.clone())
                .into_any_element(),
            Role::Error => div()
                .px_3()
                .py_2()
                .rounded(metrics::RADIUS_LG)
                .bg(t.destructive.opacity(0.12))
                .text_color(t.destructive)
                .child(m.text.clone())
                .into_any_element(),
        }
    }
}

impl Render for AssistantPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let session = self.session.read(cx);
        let busy = session.busy();
        let status = session.status().map(|s| s.to_string());
        let approval = session.pending_approval().map(|a| a.to_string());
        let messages: Vec<AnyElement> = session
            .messages()
            .iter()
            .enumerate()
            .map(|(i, m)| self.render_message(i, m, cx))
            .collect();
        let empty = messages.is_empty();
        let browser = self.browser.clone();

        v_flex()
            .w(px(360.))
            .h_full()
            .flex_none()
            .py(metrics::CARD_INSET)
            .pr(metrics::CARD_INSET)
            .child(
                v_flex()
                    .size_full()
                    .rounded(metrics::RADIUS)
                    .bg(t.card)
                    .border_1()
                    .border_color(t.border.opacity(0.7))
                    .shadow(t.card_shadow())
                    .child(
                        h_flex()
                            .h(px(48.))
                            .flex_none()
                            .px_3()
                            .gap_2()
                            .border_b_1()
                            .border_color(t.border.opacity(0.5))
                            .child(icon("sparkles", px(16.), t.primary))
                            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child("Assistant"))
                            .child(icon_button("assistant-new", "square-pen", px(28.), true, cx).on_click(
                                cx.listener(|this, _, _, cx| this.session.update(cx, |s, cx| s.reset(cx))),
                            ))
                            .child(icon_button("assistant-close", "x", px(28.), true, cx).on_click(
                                move |_, window, cx| {
                                    browser
                                        .update(cx, |b, cx| b.toggle_assistant(window, cx))
                                        .ok();
                                },
                            )),
                    )
                    .child(
                        v_flex()
                            .id("assistant-messages")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p_3()
                            .gap_3()
                            .when(empty, |s| {
                                s.child(
                                    v_flex()
                                        .gap_2()
                                        .pt_8()
                                        .items_center()
                                        .text_color(t.muted_foreground)
                                        .child(div().text_size(metrics::TEXT_TITLE).child("Ask about this page"))
                                        .child(h_flex().gap_2().flex_wrap().justify_center().children(
                                            ["Summarize", "Key points", "Tidy tabs"].into_iter().enumerate().map(
                                                |(i, label)| {
                                                    text_button(("chip", i), label, false, cx).on_click(
                                                        cx.listener(move |this, _, window, cx| {
                                                            let prompt = match label {
                                                                "Summarize" => "Summarize this page.",
                                                                "Key points" => "List the key points of this page.",
                                                                _ => "Tidy up my tabs: suggest folders and close duplicates.",
                                                            };
                                                            this.send(prompt.to_string(), window, cx);
                                                        }),
                                                    )
                                                },
                                            ),
                                        )),
                                )
                            })
                            .children(messages)
                            .when_some(status, |s, status| {
                                s.child(
                                    div()
                                        .text_size(metrics::TEXT_SMALL)
                                        .text_color(t.muted_foreground)
                                        .child(status),
                                )
                            }),
                    )
                    .when_some(approval, |s, text| {
                        s.child(
                            v_flex()
                                .m_3()
                                .p_3()
                                .gap_2()
                                .rounded(metrics::RADIUS_POPOVER)
                                .border_1()
                                .border_color(t.status_warning.opacity(0.6))
                                .bg(t.status_warning.opacity(0.08))
                                .child(div().font_weight(FontWeight::MEDIUM).child("Allow this action?"))
                                .child(div().text_size(metrics::TEXT_LABEL).child(text))
                                .child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(text_button("deny", "Deny", false, cx).on_click(cx.listener(
                                            |this, _, _, cx| this.session.update(cx, |s, cx| s.resolve_approval(false, cx)),
                                        )))
                                        .child(text_button("allow", "Allow", true, cx).on_click(cx.listener(
                                            |this, _, _, cx| this.session.update(cx, |s, cx| s.resolve_approval(true, cx)),
                                        ))),
                                ),
                        )
                    })
                    .child(
                        h_flex()
                            .m_3()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.input)))
                            .when(busy, |s| {
                                s.child(icon_button("assistant-stop", "square", px(28.), true, cx).on_click(
                                    cx.listener(|this, _, _, cx| this.session.update(cx, |s, cx| s.interrupt(cx))),
                                ))
                            }),
                    ),
            )
    }
}
