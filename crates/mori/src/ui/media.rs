//! Background media: which tabs play sound (the tab row's speaker), and the
//! sidebar mini player shown while media plays in a tab other than the
//! selected one.
//!
//! A tab's media is read with a page script, never as a user gesture, and
//! only when there is reason to: shortly after the page loads or the user
//! clicks or presses a command key in it, then every second while it plays
//! (for the mini player's progress) and every few seconds while paused.

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

/// Polls between checks of a tab whose media is paused.
const PAUSED_POLL_TICKS: u64 = 5;

/// Delay before reading a tab's media after a load or a click, so playback
/// has started (or stopped) by then.
const CHECK_DELAY: Duration = Duration::from_millis(900);

/// Whether a tab is making sound: its media plays unmuted and the tab is not
/// muted.
pub fn is_audible(state: &MediaState, tab_muted: bool) -> bool {
    state.has_media && state.playing && !state.muted && !tab_muted
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
            let mut tick: u64 = 0;
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                tick += 1;
                if this.update(cx, |this, cx| this.poll_media(tick, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Re-read tabs known to have media; tabs without any are left alone
    /// until a load or a click gives reason to look.
    fn poll_media(&mut self, tick: u64, cx: &mut Context<Self>) {
        self.media.retain(|id, _| self.live.contains_key(id));
        let due: Vec<TabId> = self
            .media
            .iter()
            .filter(|(_, m)| m.state.playing || tick.is_multiple_of(PAUSED_POLL_TICKS))
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            self.check_media(id, cx);
        }
    }

    /// Read a tab's media state shortly (see [`CHECK_DELAY`]).
    pub fn check_media_soon(&mut self, id: TabId, cx: &mut Context<Self>) {
        if !self.media_checks.insert(id) {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CHECK_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.media_checks.remove(&id);
                this.check_media(id, cx);
            });
        })
        .detach();
    }

    fn check_media(&mut self, id: TabId, cx: &mut Context<Self>) {
        let Some(live) = self.live.get(&id) else { return };
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

    /// The tab is playing sound right now (the tab row shows a speaker).
    pub fn is_tab_audible(&self, id: TabId) -> bool {
        let tab_muted = self.live.get(&id).is_some_and(|l| l.muted);
        self.media.get(&id).is_some_and(|m| is_audible(&m.state, tab_muted))
    }

    fn set_media(&mut self, id: TabId, state: MediaState, cx: &mut Context<Self>) {
        let changed = self
            .media
            .get(&id)
            .map(|m| (m.state.playing, m.state.muted, m.state.has_media, m.state.title.clone()))
            != Some((state.playing, state.muted, state.has_media, state.title.clone()));
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

    pub fn media_command(&mut self, id: TabId, command: MediaCommand, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get(&id) {
            // The user pressed a media button: play() may need activation.
            live.view.evaluate_as_user(&media_command_script(command), |_| {});
            self.check_media_soon(id, cx);
        }
    }

    /// The mini player at the bottom of the sidebar (Arc's): the site's
    /// icon and what plays, play/pause, skip, mute and Picture-in-Picture;
    /// clicking the title jumps to the tab.
    pub fn render_media_strip(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.media_source()?;
        let media = self.media.get(&id)?.state.clone();
        let t = cx.tokens().clone();
        let record = self.ws.tab(id);
        let tab_title = record.map(|r| r.display_title().to_string()).unwrap_or_default();
        let host = record.and_then(|r| mori_core::util::host_of(&r.url));
        let favicon = host.as_deref().and_then(|h| self.favicons.get(h));
        let title = if media.title.is_empty() { tab_title } else { media.title.clone() };
        let muted = media.muted || self.live.get(&id).is_some_and(|l| l.muted);
        let in_pip = self.pip_tab() == Some(id);
        let progress = if media.duration > 0.0 {
            (media.position / media.duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let button = |name: &'static str, glyph: &'static str, size: Pixels, cx: &App| icon_button(name, glyph, size, true, cx);
        Some(
            v_flex()
                .id("media-strip")
                .mx(px(8.))
                .mb_1()
                .p(px(8.))
                .gap(px(6.))
                .rounded(metrics::RADIUS_POPOVER)
                .bg(t.foreground.opacity(0.06))
                .border_1()
                .border_color(t.foreground.opacity(0.05))
                .child(
                    h_flex()
                        .id("media-title")
                        .gap_2()
                        .cursor_pointer()
                        .child(favicon_el(favicon, cx))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().truncate().font_weight(FontWeight::MEDIUM).child(title))
                                .when(!media.artist.is_empty(), |s| {
                                    s.child(
                                        div()
                                            .truncate()
                                            .text_size(metrics::TEXT_SMALL)
                                            .text_color(t.muted_foreground)
                                            .child(media.artist.clone()),
                                    )
                                }),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| this.select_tab(id, window, cx))),
                )
                .child(
                    h_flex()
                        .gap(px(2.))
                        .child(button("media-back", "rotate-ccw", px(26.), cx).on_click(cx.listener(
                            move |this, _, _, cx| this.media_command(id, MediaCommand::SeekBy(-10.0), cx),
                        )))
                        .child(
                            button("media-play", if media.playing { "pause" } else { "play" }, px(28.), cx)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.media_command(id, MediaCommand::TogglePlay, cx)
                                })),
                        )
                        .child(button("media-fwd", "rotate-cw", px(26.), cx).on_click(cx.listener(
                            move |this, _, _, cx| this.media_command(id, MediaCommand::SeekBy(10.0), cx),
                        )))
                        .child(div().flex_1())
                        .child(
                            button("media-mute", if muted { "volume-x" } else { "volume-2" }, px(26.), cx)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_mute(id, cx);
                                    this.check_media_soon(id, cx);
                                })),
                        )
                        .when(media.is_video, |row| {
                            row.child(
                                button("media-pip", "picture-in-picture-2", px(26.), cx)
                                    .when(in_pip, |b| b.bg(t.foreground.opacity(0.1)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.pip_tab() == Some(id) {
                                            this.close_pip(false, cx);
                                        } else {
                                            this.open_pip(id, true, cx);
                                        }
                                    })),
                            )
                        }),
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

fn favicon_el(image: Option<std::sync::Arc<RenderImage>>, cx: &App) -> AnyElement {
    crate::ui::components::favicon(image, px(16.), false, cx)
}

#[cfg(test)]
mod tests {
    use super::{MediaState, is_audible};

    #[test]
    fn audible_only_when_playing_unmuted() {
        let playing = MediaState { has_media: true, playing: true, ..Default::default() };
        assert!(is_audible(&playing, false));
        assert!(!is_audible(&playing, true));
        assert!(!is_audible(&MediaState { muted: true, ..playing.clone() }, false));
        assert!(!is_audible(&MediaState { playing: false, ..playing }, false));
        assert!(!is_audible(&MediaState::default(), false));
    }
}
