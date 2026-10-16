# Mori Architecture

Mori exists in two implementations that share one product design:

- **Mori 3** - Rust. A GPUI chrome around Chromium embedded through CEF's
  offscreen rendering. Cross-platform; Linux is the first target.
- **Mori 2** - macOS. A SwiftUI/AppKit chrome compiled into the Chromium
  `chrome` target (described in the second half of this document).

## Mori 3: GPUI + CEF

### Layers

```text
crates/mori         GPUI chrome: window, sidebar, launcher, panels, overlays,
                    commands, engine hub, web surfaces          (Rust, GPUI)
crates/mori-core    Product model: workspace (spaces/folders/tabs), stores,
                    omnibox, launcher ranking, routes, boosts,
                    theme engine, page scripts                  (pure Rust)
crates/mori-engine  Chromium engine: CEF init, offscreen browsers, input,
                    downloads, dialogs, permissions, DevTools  (Rust over C++ CEF)
Chromium (CEF 154)  renderer, network, storage, media, GPU     (C++)
```

`mori-core` knows nothing about GPUI or CEF, so every product rule (tab
placement, Air Traffic Control matching, launcher scoring, URL-vs-search
heuristics, theme maths) is unit-tested without a window.

### Rendering a page

Every tab is a CEF browser created with windowless (offscreen) rendering:

1. `mori-engine` reports the view size and device scale to Chromium
   (`RenderHandler::view_rect` / `screen_info`).
2. Chromium paints BGRA frames into `on_paint`; the engine keeps the latest
   frame and bumps a generation counter.
3. `EngineHub` runs on GPUI's foreground executor. It calls CEF's
   `do_message_loop_work` (CEF is initialised with `external_message_pump`,
   so all CEF callbacks happen inside that call on the UI thread), then routes
   each dirty view to its `WebSurface`.
4. `WebSurface` turns the frame into a GPUI `RenderImage` and paints it with
   the web card's rounded corners. `<select>` dropdowns arrive as a separate
   popup paint and are drawn on top.

Input flows the other way: `WebSurface` registers window-level mouse
listeners and translates GPUI mouse, wheel and key events into CEF events.
Key bindings resolve first, so Mori's shortcuts win; everything else goes to
the page. Typed text arrives through GPUI's input handler (IME-aware) and is
sent as `KEYEVENT_CHAR`s or IME composition calls, never twice.

### Events and state

CEF handlers (`client.rs`) hold weak references to their view and push
`ViewEvent`s (address, title, favicon, loading, popups, downloads, dialogs,
permissions, find results, context menus, fullscreen). `Browser::on_view_events`
applies them to the workspace and the per-tab `LiveTab` runtime state.

- Popups (`target=_blank`, `window.open`) are allowed to be created by
  Chromium so `window.opener` keeps working, but they render through a Mori
  client and arrive as `ViewEvent::NewView`; the chrome turns them into tabs.
- Script results (`WebView::evaluate`) travel over the DevTools protocol, so
  page script cannot forge them. Reader mode, Boost zapping, and the assistant
  tools use this.
- Context menus are suppressed in Chromium and drawn by Mori from
  `ContextMenuInfo`.

### Ownership rules

- Chromium owns pages, networking, storage, permissions enforcement, media,
  and downloads I/O.
- `mori-engine` stays a thin, safe Rust surface over CEF. It holds no product
  rules.
- `mori-core` owns product rules and persisted state.
- `crates/mori` owns presentation and wires the other two together.

### Processes

Chromium re-executes the `mori` binary for renderer, GPU, and utility
processes; `main` calls `mori_engine::run_subprocess_if_needed()` before
anything else. A second `mori` launch with the same profile is forwarded to
the running instance by Chromium's process singleton
(`on_already_running_app_relaunch`): its URLs open in Little Mori (or the
current space), and `--new-window` / `--private-window` open windows.

### Windows and profiles

`crates/mori/src/windows.rs` opens and tracks the windows. Each window's
root `Browser` holds its own copy of the stores; when one notifies, what it
shares with each other window is compared and copied (normal windows share
everything, Little Mori shares all but its one-page workspace, private
windows only preferences). `mori_core::windows::adopt_workspace` keeps each
window's own selection over the shared tabs, and one window writes the
stores. A tab's running page lives in one window and moves when another
window shows it; a window whose page went away (its window closed, or the
page slept) loads it again. A run that starts with Little Mori or a private
window restores the saved windows when Mori proper first opens.

Pages run in a `mori_engine::BrowsingContext`: CEF's global context, a
persistent context per profile (`<profile root>/Profile-<id>`, assigned to
spaces in `mori_core::profiles`), or an in-memory context shared by the open
private windows. Little Mori opens its page in the profile of the space
"Open in …" would take it to.

## Mori 2: SwiftUI on Chromium (macOS)

Mori 2 is a native macOS browser chrome embedded in the Chromium `chrome` target.
The boundary is intentionally simple: Chromium owns browser primitives, while
Mori owns the native chrome and the bridge layer that asks Chromium to act.

### Runtime Layers

```text
SwiftUI chrome
  RootView, Sidebar, Toolbar, LauncherOverlay, panels, themes, stores

Objective-C++ bridge
  mori_chrome_bridge.mm, mori_browser_window.mm, extension and permission glue

Chromium browser stack
  Browser, WebContents, tabs, navigation, renderers, extensions, downloads
```

### Ownership Rules

- Swift files under `chrome/browser/ui/mori` own visible Mori UI, local app
  state, shortcut routing, settings, and feature models.
- Objective-C++ files under the same directory translate Mori actions into
  Chromium browser commands and forward Chromium state back into Swift.
- Chromium continues to own page loading, renderer lifecycle, permissions,
  downloads, extensions, session primitives, and web text input.
- The bridge should stay thin. If logic is UI state or product behavior, keep it
  in Swift. If logic requires Chromium browser objects, keep it in Objective-C++.

### Main Data Flow

1. `mori_browser_window.mm` installs Mori into the Chromium browser window.
2. `MoriRoot.swift` creates the SwiftUI root view and the shared `BrowserStore`.
3. SwiftUI views mutate `BrowserStore` in response to user actions.
4. `mori_chrome_bridge.mm` converts those actions into Chromium operations.
5. Bridge callbacks update Swift stores so the chrome reflects Chromium state.

### Keyboard Flow

Keyboard handling is shared between AppKit and Chromium web-view pre-handling:

- `ShortcutRegistry.swift` defines the single command routing table.
- `MoriRoot.swift` exposes Objective-C entry points for shortcut handling.
- `mori_browser_window.mm` forwards web-focused key events before Chromium
  consumes them.
- Accepted shortcuts force a SwiftUI redraw pulse so state changes become
  visible immediately under Chromium's macOS message pump.

### Feature Boundaries

- Tabs, contexts, folders, sleep, and archive state: `BrowserStore.swift`,
  `BrowserTab.swift`, `Contexts.swift`, `TabFolder.swift`,
  `TabMaintenance.swift`, and `ArchiveStore.swift`.
- Command palette and shortcuts: `LauncherOverlay.swift` and
  `ShortcutRegistry.swift`.
- AI panel and page automation: `AIPanel.swift`, `CodexAppServerClient.swift`,
  and `BrowserAutomation.swift`.
- Per-site customization: `Boosts.swift`, `BoostStore.swift`, `Reader.swift`,
  `Peek.swift`, `AirTrafficControl.swift`, and `RouteStore.swift`.
- Chromium integration: `mori_chrome_bridge.mm`, `mori_browser_window.mm`,
  `mori_chrome_extensions.mm`, `mori_permission_prompt.mm`, and bridge headers.

### Structural Notes

The Mori source directory is currently flat because Chromium's GN target lists
Swift sources explicitly in `chrome/browser/ui/BUILD.gn`. Moving files into
subdirectories is possible, but should be done as a separate build-verified
change that updates the GN source list at the same time.
