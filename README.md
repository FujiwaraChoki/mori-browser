<div align="center">

# 🌲 Mori

**A calm browser chrome, built on Chromium.**

Mori wraps a real Chromium engine in a hand-built interface: spaces, an AI side
panel, per-site tweaks, gradient theming, and a command-palette launcher, while
leaving rendering, networking, downloads, and permissions to Chromium.

**Mori 3** is written in Rust with [GPUI](https://www.gpui.rs) and embeds
Chromium through CEF, so it runs on Linux. **Mori 2** is the original macOS
SwiftUI chrome compiled into ungoogled-chromium.

</div>

---

## What Mori is

Mori is **not** a fork of Chromium's UI. Chromium owns the web; Mori owns
everything around it.

Mori 3 (Rust, `crates/`):

```
┌─────────────────────────────────────────────┐
│  crates/mori        GPUI chrome: sidebar,     │
│                     launcher, panels, themes  │
├─────────────────────────────────────────────┤
│  crates/mori-core   spaces, stores, omnibox,  │
│                     ranking, theme engine     │
├─────────────────────────────────────────────┤
│  crates/mori-engine CEF offscreen browsers,   │
│                     input, events             │
├─────────────────────────────────────────────┤
│  Chromium (CEF)     renderer, network, media  │
└─────────────────────────────────────────────┘
```

Mori 2 (macOS) is a custom SwiftUI + Objective-C++ chrome compiled *into* the
`chrome` target of an
[ungoogled-chromium](https://github.com/ungoogled-software/ungoogled-chromium)
checkout:

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

| Feature | Description |
|---|---|
| 🗂️ **Spaces & folders** | Arc-style contexts and tab folders with custom glyph icons. |
| ✈️ **Air Traffic Control** | Routing rules auto-file tabs into the space that matches their host. |
| 🤖 **AI panel** | A side panel backed by a local Codex app server, with tools to read pages and act like a user. |
| ⚡ **Boosts** | Per-site custom CSS/JS and click-to-zap element removal, matched by host suffix. |
| 👁️ **Peek** | A floating card that hosts a transient tab. Promote it to a real tab or dismiss it. |
| 📖 **Reader** | Distraction-free reading mode. |
| 🎨 **Gradient themes** | Wash the chrome in a custom hue/saturation gradient that derives a matching accent. |
| 🔍 **Launcher** | A Spotlight-style ⌘T command palette to search, jump to open tabs, or pick from history. |
| 💤 **Sleep & archive** | Stale tabs sleep to save memory and archive restorably instead of vanishing. |
| 🔑 **Passkeys, downloads, media** | Native passkey auth, a downloads panel, and picture-in-picture media controls. |

## Security status

Mori is still in its infancy and should be treated as experimental software.
Security risk is especially relevant around features that execute or interpret
site content, including Boosts, injected CSS/JavaScript, AI page-reading tools,
and prompt-injection paths through web content. Use a dedicated test profile,
avoid sensitive browsing, and review any site automation or custom scripts
before trusting them.

## Documentation map

Start here, then use the focused docs when you need the exact workflow:

- [BUILDING.md](BUILDING.md) - local build, package, run, and logging commands.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) - how Mori's SwiftUI chrome,
  Objective-C++ bridge, and Chromium browser primitives fit together.
- [docs/SOURCE_LAYOUT.md](docs/SOURCE_LAYOUT.md) - where the first-party code
  lives and which files own each feature area.

## Feature status in Mori 3

Implemented: spaces with per-space themes, pinned tiles, folders, drag and
drop, split view, the launcher (tabs, history, bookmarks, commands, search),
Air Traffic Control, Boosts (custom CSS/JS and element zapping), Peek, Reader,
find in page, sleep and archive timers, the background media mini player,
downloads, the history/bookmarks/archive library, gradient themes and presets,
JavaScript dialogs, permission prompts, custom context menus, DevTools,
screenshots, session restore, single-instance URL hand-off, a system clipboard
bridge, and the assistant panel (a local Codex app server with browser tools
and per-action approval).

Not yet ported from Mori 2: Chrome extensions, passkeys, automatic
Picture-in-Picture, the back/forward history menu, and native drag sessions out
of web pages.

## Source layout

Mori 3 lives in the Cargo workspace under `crates/` (see
[docs/SOURCE_LAYOUT.md](docs/SOURCE_LAYOUT.md)).

The Mori 2 source of truth lives in:

```
ungoogled-chromium-macos/build/src/chrome/browser/ui/mori/
```

Everything else - the rest of the Chromium checkout, `depot_tools`, and build
outputs - is a local dependency and intentionally ignored by Git.

For the file-by-file map, see [docs/SOURCE_LAYOUT.md](docs/SOURCE_LAYOUT.md).

## Building

Full instructions - including packaging and logging - live in
[BUILDING.md](BUILDING.md).

Mori 3 on Linux:

```sh
cargo run --release               # downloads CEF into .cef/ on first build
scripts/package-linux.sh          # self-contained bundle in dist/
scripts/install-linux.sh          # install to ~/.local
```

Mori 2 on macOS, the short version:

```sh
repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root/ungoogled-chromium-macos/build/src"
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
export PATH="$repo_root/binshims:$repo_root/depot_tools:$PATH"

ninja -C out/Default chrome
```

A successful build produces `out/Default/Mori.app`. Treat ninja exit code `0` as
the source of truth. Swift deployment-target and `CoreAudioTypes` warnings are
expected.

> Use `trash`, not `rm -rf`, when replacing app bundles or build directories.

## Running Mori 2

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

See [BUILDING.md](BUILDING.md) for verifying that an instrumented framework
was actually packaged, and for avoiding stale already-running instances.
