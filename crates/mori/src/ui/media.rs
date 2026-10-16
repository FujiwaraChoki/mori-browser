//! Background media: a 1s poll of playing tabs and the sidebar mini player
//! shown while media plays in a tab other than the selected one.

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::{
    scripts::{MEDIA_STATE_SCRIPT, MediaCommand, media_command_script},
    workspace::TabId,
};
use serde::Deserialize;

use crate::{
    browser::Browser,
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon_button, v_flex},
};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaState {
    pub has_media: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub position: f64,
    pub duration: f64,
    pub muted: bool,
    pub is_video: bool,
}

/// The most recent media report per tab, plus when it last played.
#[derive(Clone, Debug, Default)]
pub struct TabMedia {
    pub state: MediaState,
    pub last_playing: f64,
}

impl Browser {
    pub fn start_media_polling(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |this, cx| this.poll_media(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn poll_media(&mut self, cx: &mut Context<Self>) {
        for (id, live) in &self.live {
            let id = *id;
            let weak = cx.entity().downgrade();
            let async_cx = cx.to_async();
            live.view.evaluate(MEDIA_STATE_SCRIPT, move |result| {
                let state: MediaState = result
                    .ok()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                let _ = weak.update(&mut async_cx.clone(), |this, cx| this.set_media(id, state, cx));
            });
        }
        let live: Vec<TabId> = self.live.keys().copied().collect();
        self.media.retain(|id, _| live.contains(id));
    }

    fn set_media(&mut self, id: TabId, state: MediaState, cx: &mut Context<Self>) {
        let changed = self.media.get(&id).map(|m| (m.state.playing, m.state.has_media, m.state.title.clone()))
            != Some((state.playing, state.has_media, state.title.clone()));
        if !state.has_media {
            if self.media.remove(&id).is_some() {
                cx.notify();
            }
            return;
        }
        let entry = self.media.entry(id).or_default();
        if state.playing {
            entry.last_playing = mori_core::paths::now();
        }
        entry.state = state;
        if changed || self.media_source() == Some(id) {
            cx.notify();
        }
    }

    /// The tab whose media the mini player controls: the most recently
    /// playing one that isn't on screen.
    pub fn media_source(&self) -> Option<TabId> {
        let visible = [self.selected(), self.split];
        self.media
            .iter()
            .filter(|(id, m)| !visible.contains(&Some(**id)) && m.last_playing > 0.0)
            .max_by(|a, b| a.1.last_playing.total_cmp(&b.1.last_playing))
            .map(|(id, _)| *id)
    }

    pub fn media_command(&mut self, id: TabId, command: MediaCommand) {
        if let Some(live) = self.live.get(&id) {
            live.view.evaluate(&media_command_script(command), |_| {});
        }
    }

    pub fn render_media_strip(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.media_source()?;
        let media = self.media.get(&id)?.state.clone();
        let t = cx.tokens().clone();
        let tab_title = self.ws.tab(id).map(|r| r.display_title().to_string()).unwrap_or_default();
        let title = if media.title.is_empty() { tab_title } else { media.title.clone() };
        let progress = if media.duration > 0.0 {
            (media.position / media.duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        Some(
            v_flex()
                .id("media-strip")
                .mx(px(8.))
                .mb_1()
                .p_2()
                .gap_1()
                .rounded(metrics::RADIUS_POPOVER)
                .bg(t.foreground.opacity(0.06))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            v_flex()
                                .id("media-title")
                                .flex_1()
                                .min_w_0()
                                .cursor_pointer()
                                .child(div().truncate().font_weight(FontWeight::MEDIUM).child(title))
                                .when(!media.artist.is_empty(), |s| {
                                    s.child(
                                        div()
                                            .truncate()
                                            .text_size(metrics::TEXT_SMALL)
                                            .text_color(t.muted_foreground)
                                            .child(media.artist.clone()),
                                    )
                                })
                                .on_click(cx.listener(move |this, _, window, cx| this.select_tab(id, window, cx))),
                        )
                        .child(icon_button("media-back", "rotate-ccw", px(26.), true, cx).on_click(cx.listener(
                            move |this, _, _, _| this.media_command(id, MediaCommand::SeekBy(-10.0)),
                        )))
                        .child(
                            icon_button("media-play", if media.playing { "pause" } else { "play" }, px(28.), true, cx)
                                .on_click(cx.listener(move |this, _, _, _| {
                                    this.media_command(id, MediaCommand::TogglePlay)
                                })),
                        )
                        .child(icon_button("media-fwd", "rotate-cw", px(26.), true, cx).on_click(cx.listener(
                            move |this, _, _, _| this.media_command(id, MediaCommand::SeekBy(10.0)),
                        ))),
                )
                .child(
                    div()
                        .h(px(3.))
                        .rounded_full()
                        .bg(t.foreground.opacity(0.12))
                        .child(div().h_full().rounded_full().bg(t.primary).w(relative(progress))),
                )
                .into_any_element(),
        )
    }
}
