//! Mori: a GPUI browser chrome around Chromium (CEF).

mod actions;
mod assets;
mod assistant;
mod browser;
mod commands;
mod default_browser;
mod devtools_bridge;
mod engine_hub;
mod favicons;
mod notifications;
mod passwords;
mod screenshot;
mod theme;
mod ui;
mod web_surface;
mod windows;

use mori_engine::EngineConfig;

use crate::engine_hub::EngineHub;

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
    // Profiles deleted last time go now; this instance owns the profile
    // directory, and Chromium only opens a profile once a page needs it.
    windows::purge_deleted_profiles(&data_dir, &config.profile_dir);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let private = args.iter().any(|a| a == "--private-window");
    let start_urls: Vec<String> = args.into_iter().filter(|a| !a.is_empty() && !a.starts_with("--")).collect();

    gpui_kit::application()
        .with_assets(assets::MoriAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            actions::bind_keys(cx);
            EngineHub::start(cx);
            windows::init(cx);
            windows::start(start_urls, private, cx);

            cx.on_app_quit(|_| async {
                mori_engine::close_all(std::time::Duration::from_millis(800));
                mori_engine::shutdown();
            })
            .detach();
        });
}
