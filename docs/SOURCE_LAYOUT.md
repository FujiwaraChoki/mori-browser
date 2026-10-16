# Source Layout

## Mori 3 (Rust workspace)

```text
Cargo.toml                  workspace; CEF and GPUI versions are pinned here
.cargo/config.toml          CEF_PATH -> .cef/ (downloaded on first build)
crates/mori-engine/         Chromium via CEF
crates/mori-core/           product model and logic (no UI, no engine)
crates/mori/                the GPUI application
scripts/package-linux.sh    self-contained bundle in dist/
scripts/install-linux.sh    per-user install (~/.local)
packaging/linux/            desktop entry
```

### `crates/mori-engine`

- `lib.rs` - process entry (`run_subprocess_if_needed`), `initialize`, the
  external message pump (`pump`, `next_pump_delay_ms`), shutdown, relaunch
  queue, resource discovery.
- `app.rs` - CEF `App`: command-line policy and the browser process handler.
- `context.rs` - browsing contexts: the global one, persistent profiles and
  in-memory private contexts.
- `view.rs` - `WebView` (one offscreen browser), `ViewEvent`, frames, and every
  page operation (navigation, input, editing, find, zoom, dialogs, permissions).
- `client.rs` - all CEF handlers (render, display, load, life span/popups,
  requests, JS dialogs, permissions, context menu, find).
- `downloads.rs` - download handler and controls.
- `devtools.rs` - `evaluate` over the DevTools protocol.
- `keys.rs` - key name to Windows key code / XKB scan code translation.

### `crates/mori-core`

- `workspace.rs` - spaces, folders, pinned/loose tabs, selection, moves,
  session persistence.
- `settings.rs`, `history.rs`, `bookmarks.rs`, `archive.rs`, `routes.rs`
  (Air Traffic Control), `boosts.rs` - persisted stores.
- `omnibox.rs` - URL-vs-search resolution and display helpers.
- `launcher.rs` - launcher scoring and ranking.
- `theme.rs` - palettes, OKLCH/HSL maths, gradient themes and presets.
- `scripts.rs` - page scripts (reader, media, page text).
- `paths.rs`, `util.rs` - data directory, atomic JSON I/O, host helpers.
- `windows.rs` - per-window selection over the shared workspace and the saved
  windows; `profiles.rs` - profiles and which space uses which.

### `crates/mori`

- `main.rs` - startup: subprocess check, engine init, URL arguments.
- `windows.rs` - normal, private and Little Mori windows: opening, session
  restore, keeping stores in step, relaunches, per-window themes, profiles.
- `default_browser.rs` - checking and setting the default browser.
- `browser.rs` - the `Browser` root entity: model, live tabs, engine events,
  and commands.
- `engine_hub.rs` - pumps CEF from GPUI and routes view events.
- `web_surface.rs` - draws a page and forwards mouse/keyboard/IME input.
- `actions.rs` - actions and default key bindings.
- `commands.rs` - named commands shared by the launcher and menus.
- `theme.rs` - GPUI tokens, gradient wash and grain rendering.
- `favicons.rs`, `screenshot.rs`, `assets.rs`.
- `assistant/` - the Codex app-server client and browser tools.
- `ui/` - `root.rs` (layout and action routing), `sidebar.rs`, `launcher.rs`,
  `find_bar.rs`, `menus.rs`, `overlays.rs` (dialogs, permissions, Peek),
  `panels.rs` (Downloads, Library), `settings.rs`, `boost_editor.rs`,
  `assistant.rs`, `components.rs`, `windows.rs` (Little Mori header, private
  badge, window and profile menus and settings).

## Mori 2 (macOS overlay)

This repository contains a full Chromium checkout, but the first-party Mori
code is intentionally narrow.

### Repository Root

- `README.md` - project overview and entry point.
- `BUILDING.md` - canonical build, package, run, and logging workflow.
- `docs/ARCHITECTURE.md` - runtime boundaries and data flow.
- `ungoogled-chromium-macos/build/src` - Chromium source tree used for builds.
- `depot_tools` and `binshims` - local build tooling.

Build products, the Chromium checkout, and `depot_tools` are local dependencies.
The Mori overlay under Chromium is the source of truth for app behavior.

### Mori Overlay

Path:

```text
ungoogled-chromium-macos/build/src/chrome/browser/ui/mori/
```

Do not edit older prototypes under `content/shell/browser/mori`; they are not
the app that gets packaged.

There is also a local overlay guide at:

```text
ungoogled-chromium-macos/build/src/chrome/browser/ui/mori/README.md
```

### SwiftUI Chrome

- `MoriRoot.swift` - SwiftUI root, Objective-C entry points, and the shared
  app model exposed to the bridge.
- `RootView.swift`, `Sidebar.swift`, `Toolbar.swift`, `TabRow.swift`,
  `LauncherOverlay.swift` - browser chrome, tab surfaces, and command palette.
- `Components.swift`, `Glass.swift`, `Icon.swift`, `MorphingFolderIcon.swift`,
  `ToastCenter.swift`, `ToastOverlay.swift` - shared UI building blocks.
- `SettingsView.swift`, `ThemePicker.swift`, `Theme.swift`,
  `GradientTheme.swift`, `GradientEngine.swift`, `ThemePresets.swift`,
  `OKLCH.swift`, `FontRegistry.swift` - settings, tokens, and theming.

### App State

- `BrowserStore.swift` - top-level observable browser state.
- `BrowserStore+DragDrop.swift` - drag/drop behavior for sidebar organization.
- `BrowserTab.swift`, `BrowserContext.swift`, `Contexts.swift`,
  `TabFolder.swift` - tabs, spaces, and folder models.
- `BrowserSettings.swift`, `ArchiveStore.swift`, `BookmarkStore.swift`,
  `DownloadStore.swift`, `ExtensionStore.swift`, `HistoryStore.swift` -
  persisted app and browser-adjacent stores.

### Product Features

- `AIPanel.swift`, `CodexAppServerClient.swift`, `BrowserAutomation.swift` -
  AI panel and page-acting tools.
- `Boosts.swift`, `BoostStore.swift`, `Reader.swift`, `Peek.swift`,
  `SidebarPeek.swift`, `Screenshot.swift` - page customization and transient
  browsing tools.
- `AirTrafficControl.swift`, `RouteStore.swift` - host-based tab routing.
- `DownloadsPanel.swift`, `ExtensionsMenu.swift`, `LibraryPanel.swift`,
  `FindBar.swift`, `ContextMenu.swift` - browser panels and overlays.
- `MediaAgentScripts.swift`, `MediaController.swift`, `MediaPlayer.swift`,
  `MediaPolling.swift`, `PiPWindowStyler.swift` - media detection and controls.
- `PasskeyAuthenticator.swift`, `PasskeySupport.swift` - passkey support.
- `ShortcutRegistry.swift` - app command routing and reserved Chromium shortcut
  decisions.
- `TabMaintenance.swift` - tab sleep and archive timers.

### Chromium Bridge

- `mori_bridge.h` - Swift bridging header.
- `mori_chrome_bridge.mm` - main Swift/AppKit to Chromium integration layer.
- `mori_browser_window.mm` and `mori_browser_window.h` - browser window
  lifecycle and web-focused keyboard handling.
- `mori_chrome_extensions.mm` - Chromium extension API integration.
- `mori_permission_prompt.mm` and `mori_permission_prompt.h` - permission UI
  integration.
- `MoriBrowserView.h`, `MoriPrivacy.h`, `mori_chrome_hooks.h` - remaining
  bridge declarations.

### Build Wiring

The Swift files are compiled by the `mori_ui_swift` target in:

```text
ungoogled-chromium-macos/build/src/chrome/browser/ui/BUILD.gn
```

When adding, moving, or deleting Swift files, update that GN source list in the
same change.
