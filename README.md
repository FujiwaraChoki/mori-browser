<div align="center">

# 🌲 Mori

**A native macOS browser chrome, built on Chromium.**

Mori wraps a real Chromium engine in a hand-built SwiftUI interface — spaces, an
AI side panel, per-site tweaks, gradient theming, and a command-palette launcher
— while leaving tabs, navigation, downloads, extensions, and permissions to
Chromium's production browser stack.

</div>

---

## What Mori is

Mori is **not** a fork of Chromium's UI and **not** a CEF app. It's a custom
SwiftUI + Objective-C++ chrome compiled *into* the `chrome` target of an
[ungoogled-chromium](https://github.com/ungoogled-software/ungoogled-chromium)
checkout. Chromium owns the web; Mori owns everything around it.

```
┌─────────────────────────────────────────────┐
│  Mori chrome  (SwiftUI sidebar, launcher,    │
│               panels, theming, shortcuts)    │
├─────────────────────────────────────────────┤
│  Bridge       (Objective-C++: commands,      │
│               tabs, windows, downloads…)     │
├─────────────────────────────────────────────┤
│  Chromium     (renderer, navigation,         │
│               extensions, permissions)       │
└─────────────────────────────────────────────┘
```

## Highlights

| | |
|---|---|
| 🗂️ **Spaces & folders** | Arc-style contexts and tab folders with custom glyph icons. |
| ✈️ **Air Traffic Control** | Routing rules auto-file tabs into the space that matches their host. |
| 🤖 **AI panel** | A side panel backed by a local Codex app server, with tools to read pages and act like a user. |
| ⚡ **Boosts** | Per-site custom CSS/JS and click-to-zap element removal, matched by host suffix. |
| 👁️ **Peek** | A floating card that hosts a transient tab — promote it to a real tab or dismiss it. |
| 📖 **Reader** | Distraction-free reading mode. |
| 🎨 **Gradient themes** | Wash the chrome in a custom hue/saturation gradient that derives a matching accent. |
| 🔍 **Launcher** | A Spotlight-style ⌘T command palette to search, jump to open tabs, or pick from history. |
| 💤 **Sleep & archive** | Stale tabs sleep to save memory and archive (restorably) instead of vanishing. |
| 🔑 **Passkeys, downloads, media** | Native passkey auth, a downloads panel, and picture-in-picture media controls. |

## Source layout

The Mori source of truth lives in:

```
ungoogled-chromium-macos/build/src/chrome/browser/ui/mori/
```

Everything else — the rest of the Chromium checkout, `depot_tools`, and build
outputs — is a local dependency and intentionally ignored by Git.

> **Note:** the copy under `content/shell/browser/mori/` is an older
> content_shell prototype. Editing it will **not** change the app.

### Key files

**SwiftUI chrome**
- `MoriRoot.swift` — SwiftUI root, shortcut entry points, and the shared app model exposed to the bridge.
- `RootView.swift`, `Sidebar.swift`, `Toolbar.swift`, `LauncherOverlay.swift` — the browser chrome and command palette.
- `AIPanel.swift`, `CodexAppServerClient.swift`, `BrowserAutomation.swift` — the AI assistant and its page-acting tools.
- `Theme.swift`, `GradientTheme.swift`, `GradientEngine.swift`, `OKLCH.swift` — the design-token system and gradient theming.
- `Boosts.swift` / `BoostStore.swift`, `Peek.swift`, `Reader.swift`, `AirTrafficControl.swift` / `RouteStore.swift` — the signature features.

**State model**
- `BrowserStore.swift`, `BrowserTab.swift`, `TabFolder.swift`, `Contexts.swift`, `ArchiveStore.swift`, `ExtensionStore.swift`, `DownloadStore.swift`, `HistoryStore.swift`, `BookmarkStore.swift`.

**Shortcuts**
- `ShortcutRegistry.swift` — the single routing layer shared by AppKit event monitoring and Chromium web-view key pre-handling.

**Objective-C++ bridge**
- `mori_chrome_bridge.mm` — connects SwiftUI/AppKit chrome to Chromium browser commands, tabs, windows, menus, downloads, and extension state.
- `mori_browser_window.mm/.h` — hooks Mori into the Chromium browser window lifecycle and web-focused keyboard handling.
- `mori_chrome_extensions.mm` — integrates with Chromium's real extension APIs.
- `mori_permission_prompt.mm`, `MoriBrowserView.h`, `MoriPrivacy.h`, `mori_bridge.h` — the remaining integration surface.

## Building

Full instructions — including packaging and logging — live in
[`BUILDING.md`](BUILDING.md). The short version:

```sh
cd ungoogled-chromium-macos/build/src
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
export PATH="$HOME/Developer/chromium-mori/binshims:$HOME/Developer/chromium-mori/depot_tools:$PATH"

ninja -C out/Default chrome
```

A successful build produces `out/Default/Mori.app`. Treat ninja exit code `0` as
the source of truth — Swift deployment-target and `CoreAudioTypes` warnings are
expected.

> Use `trash`, not `rm -rf`, when replacing app bundles or build directories.

## Running

```sh
open -n out/Default/Mori.app
```

For a clean, isolated profile:

```sh
profile="$HOME/Library/Application Support/MoriBrowserTestProfile"
mkdir -p "$profile"
open -n out/Default/Mori.app --args --user-data-dir="$profile" --no-first-run
```

Session state persists to `~/Library/Application Support/MoriBrowser/session.json`.

## Debugging

Keyboard-shortcut diagnostics emit `MORI-KEY` log lines from the shortcut
registry, the Swift store, Chromium web-view pre-handling, and the AppKit event
monitor. Launch with stderr logging to see them:

```sh
out/Default/Mori.app/Contents/MacOS/Mori --enable-logging=stderr --v=0
```

See [`BUILDING.md`](BUILDING.md) for verifying that an instrumented framework
was actually packaged, and for avoiding stale already-running instances.
