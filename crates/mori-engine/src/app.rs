//! The process-wide CEF `App`: command-line policy and the browser process
//! handler that drives Mori's external message pump.

use cef::*;

#[derive(Clone)]
pub(crate) struct MoriApp;

wrap_app! {
    pub(crate) struct AppBuilder {
        app: MoriApp,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            let Some(command_line) = command_line else { return };
            // Only the browser process (empty process type) gets UI policy.
            let is_browser = process_type.map(|t| t.to_string().is_empty()).unwrap_or(true);
            if !is_browser {
                return;
            }
            let switch = |name: &str| command_line.append_switch(Some(&CefString::from(name)));
            switch("hide-crash-restore-bubble");
            switch("disable-session-crashed-bubble");
            switch("noerrdialogs");
            // Mori draws its own chrome; keep Chromium from trying to use the
            // system keychain prompt flow on Linux desktops without one.
            switch("password-store=basic");
            // Offscreen pages need no display connection: Mori draws them and
            // bridges the clipboard itself (Chromium's X11 clipboard does not
            // work under the external message pump). GPU acceleration is
            // unaffected.
            let platform =
                std::env::var("MORI_OZONE_PLATFORM").unwrap_or_else(|_| "headless".into());
            command_line.append_switch_with_value(
                Some(&CefString::from("ozone-platform")),
                Some(&CefString::from(platform.as_str())),
            );
            // Smooth scrolling and overlay scrollbars match Mori's chrome.
            switch("enable-smooth-scrolling");
            command_line.append_switch_with_value(
                Some(&CefString::from("enable-features")),
                Some(&CefString::from("OverlayScrollbar")),
            );
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(BrowserProcessHandlerBuilder::new(MoriBrowserProcess))
        }
    }
}

pub(crate) fn build_app() -> App {
    AppBuilder::new(MoriApp)
}

#[derive(Clone)]
pub(crate) struct MoriBrowserProcess;

wrap_browser_process_handler! {
    pub(crate) struct BrowserProcessHandlerBuilder {
        handler: MoriBrowserProcess,
    }

    impl BrowserProcessHandler {
        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            // May be called from any thread; it only records a deadline.
            crate::schedule_pump(delay_ms);
        }

        fn on_already_running_app_relaunch(
            &self,
            command_line: Option<&mut CommandLine>,
            _current_directory: Option<&CefString>,
        ) -> ::std::os::raw::c_int {
            // A second `mori` was started with the same profile: Chromium's
            // process singleton hands its command line to us instead.
            let mut urls = Vec::new();
            if let Some(cmd) = command_line {
                let mut args = CefStringList::new();
                cmd.arguments(Some(&mut args));
                urls = args.into_iter().filter(|a| !a.starts_with("--")).collect();
            }
            crate::push_relaunch(urls);
            1
        }
    }
}
