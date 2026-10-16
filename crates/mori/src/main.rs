//! Mori: a GPUI browser chrome around Chromium (CEF).

mod actions;
mod assets;
mod assistant;
mod browser;
mod commands;
mod devtools_bridge;
mod engine_hub;
mod favicons;
mod screenshot;
mod theme;
mod ui;
mod web_surface;

use gpui_kit::*;
use mori_engine::EngineConfig;

use crate::{browser::Browser, engine_hub::EngineHub};

fn main() {
    // Chromium re-executes this binary for renderer/GPU/utility processes.
    if let Some(code) = mori_engine::run_subprocess_if_needed() {
        std::process::exit(code);
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let data_dir = mori_core::paths::data_dir();
    let config = EngineConfig {
        profile_dir: data_dir.join("profile"),
        downloads_dir: mori_core::paths::downloads_dir(),
        user_agent_product: None,
        remote_debugging_port: std::env::var("MORI_REMOTE_DEBUGGING_PORT")
            .ok()
            .and_then(|p| p.parse().ok()),
        locale: std::env::var("LANG")
            .ok()
            .and_then(|l| l.split('.').next().map(|s| s.replace('_', "-")))
            .filter(|l| !l.is_empty() && l != "C" && l != "POSIX")
            .unwrap_or_else(|| "en-US".into()),
        no_sandbox: std::env::var_os("MORI_NO_SANDBOX").is_some(),
    };
    match mori_engine::initialize(&config) {
        Ok(mori_engine::InitOutcome::Started) => {}
        // A running Mori took our URLs and will show them.
        Ok(mori_engine::InitOutcome::HandedOff) => std::process::exit(0),
        Err(err) => {
            eprintln!("mori: {err:#}");
            std::process::exit(1);
        }
    }
    let start_urls: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .collect();

    gpui_kit::application()
        .with_assets(assets::MoriAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            actions::bind_keys(cx);
            EngineHub::start(cx);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
                app_id: Some("mori".into()),
                titlebar: Some(TitlebarOptions {
                    title: Some("Mori".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_decorations: Some(WindowDecorations::Client),
                window_min_size: Some(size(px(640.), px(420.))),
                ..Default::default()
            };
            let (window, browser) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Browser::new(window, cx))
            })
            .expect("failed to open window");
            let open_urls = move |urls: Vec<String>, cx: &mut App| {
                let browser = browser.clone();
                window
                    .update(cx, |_, window, cx| {
                        browser.update(cx, |b, cx| b.open_external_urls(&urls, window, cx));
                        window.activate_window();
                    })
                    .ok();
            };
            open_urls(start_urls.clone(), cx);
            EngineHub::on_relaunch(cx, open_urls);

            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.on_app_quit(|_| async {
                mori_engine::close_all(std::time::Duration::from_millis(800));
                mori_engine::shutdown();
            })
            .detach();
        });
}
