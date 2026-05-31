# Mori

A native macOS AI browser: **SwiftUI + AppKit** chrome wrapping a real
**Chromium** engine via **CEF** (Chromium Embedded Framework), styled to the
Mori design system, with a right-hand vertical tab sidebar by default.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ main.mm (browser process)                                    │
│  • loads the embedded CEF framework (dynamic, not linked)    │
│  • CefInitialize → CefRunMessageLoop (drives AppKit too)     │
│  • MoriApplication : NSApplication <CefAppProtocol>          │
│  • AppDelegate builds the NSWindow + menu bar, hosts SwiftUI │
└─────────────────────────────────────────────────────────────┘
        │ NSHostingController(RootView)              ▲ @objc MoriRoot
        ▼                                            │
┌──────────────────────────┐   ObjC bridge   ┌───────────────────────┐
│ SwiftUI chrome           │ ◀────────────▶  │ MoriBrowserView       │
│  RootView / Toolbar /    │  (pure-ObjC hdr)│  : NSView, hosts one    │
│  Sidebar / AIPanel /     │                 │  CEF browser per tab.   │
│  SettingsView            │                 │  BrowserClient (C++)    │
│  Theme (Mori tokens)     │                 │  forwards nav/display   │
└──────────────────────────┘                 │  state back to Swift.   │
                                             └───────────────────────┘
```

- **Engine:** CEF 148 / Chromium 148 (`third_party/cef`, arm64).
- **Process model:** the standard CEF macOS layout — the main app plus 5 helper
  app bundles (`Mori Helper`, ` (GPU)`, ` (Plugin)`, ` (Renderer)`,
  ` (Alerts)`) embedded in `Contents/Frameworks`. Helpers and the main app load
  the framework dynamically through `libcef_dll_wrapper` (the dylib variant — it
  exports `cef_load_library` and the `cef_*` C API as dlsym-backed stubs, so the
  framework is **never linked**, only embedded).
- **Chrome ↔ engine bridge:** Swift talks only to the pure-ObjC
  `MoriBrowserView` header (`Sources/Bridge`). All C++/CEF lives in `.mm`
  implementations. AppKit calls into Swift via the generated `Mori-Swift.h`
  (`@objc MoriRoot`).
- **Design system:** `Sources/UI/Theme` contains Mori's color tokens
  (OKLCH→sRGB at runtime), radii (0.4rem base), motion (snappy easing), and
  Apple **Liquid Glass** (macOS 26 `.glassEffect` + behind-window
  `NSVisualEffectView` for the translucent sidebar/panels).

## Build & run

Requirements: macOS 26+, Xcode 26+, `xcodegen` and `cmake` (Homebrew).

```bash
./run.sh            # generate project, build Debug, launch
./run.sh --release  # Release build
./run.sh --gen      # regenerate Mori.xcodeproj only
```

The first run builds `libcef_dll_wrapper` from the bundled CEF distribution.

### Notes

- The build is **ad-hoc signed** for local dev, with no personal Apple
  Developer Team baked into the project. Because each ad-hoc rebuild changes
  the signature, Chromium's Keychain "Safe Storage" would re-prompt every
  launch — we pass `--use-mock-keychain` / `--password-store=basic` by default.
  Set `MORI_USE_REAL_KEYCHAIN=1` only for a stable Developer ID build.
- Hardened runtime is **off** for dev; the entitlements still grant JIT /
  unsigned-executable-memory / library-validation-disabled for Chromium's V8.

## Local AI assistant

The Codex-powered assistant is **enabled by default**. It launches a local
Codex app server with broad filesystem access for browser automation, so run
Mori only on machines where you trust that local Codex setup. To opt out,
launch with `MORI_ENABLE_CODEX_ASSISTANT=0`. Dynamic browser tools can
additionally be enabled with `MORI_CODEX_DYNAMIC_TOOLS=1`.

## Extension readiness

Mori targets the modern Chromium extension surface: Manifest V3 manifests,
service-worker-style background runners, packaged extension pages, content
scripts, runtime messaging/ports, storage, permissions, tabs/windows, scripting,
declarativeNetRequest/webRequest events, side panels, offscreen documents,
identity redirects, cookies/history/bookmarks/downloads/sessions/management,
native messaging, CRX/Web Store install, and `browser.*` namespace aliases.

The production gate is:

```bash
./script/build_and_run.sh --verify-extension-smoke
```

That smoke boots the bundled CEF runtime, proves no external Google Chrome
process or `--load-extension` surface is used, installs multiple fixture
extensions, and exercises the extension APIs above end to end. The smoke wait
budget defaults to three minutes and can be adjusted for slow machines with
`MORI_EXTENSION_SMOKE_WAIT_ATTEMPTS` (four attempts per second).

## Keyboard shortcuts

| Shortcut | Action | Shortcut | Action |
|---|---|---|---|
| ⌘T / ⌘W | New / close tab | ⌘L | Focus omnibox |
| ⇧⌘T | Reopen closed tab | ⌘[ / ⌘] | Back / forward |
| ⌘R / ⇧⌘R | Reload / force reload | ⌘+ / ⌘- / ⌘0 | Zoom in / out / reset |
| ⌘S / ⌃S | Toggle tab sidebar | ⌘K | Toggle AI panel |
| ⌘, | Settings | ⇧⌘H | Home |

## Layout

```
Sources/
  App/       browser-process entry, CefApp/Client, NSApplication, menu, window
  Bridge/    MoriBrowserView (Swift-facing NSView over one CEF browser)
  Helper/    CEF sub-process entry point
  UI/        SwiftUI: Theme/, Models/, Views/, MoriRoot
  Resources/ Info.plists, entitlements
Scripts/embed_cef_framework.sh   versioned-framework embed + sign
third_party/cef/                 CEF 148 distribution + built wrapper
```

## License

Mori's first-party source code is MIT licensed. Third-party components and
assets keep their own terms; see `THIRD_PARTY_NOTICES.md`.
