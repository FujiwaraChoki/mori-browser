# Mori Browser

Mori is now a custom macOS browser UI embedded directly into
ungoogled-chromium. This repository used to contain a standalone CEF app; the
current architecture is a Chromium overlay that replaces the old CEF runtime,
helper app, and extension shims with Chromium's real browser stack.

## Current Architecture

The Mori source of truth lives in:

```text
ungoogled-chromium-macos/build/src/chrome/browser/ui/mori
```

That directory is compiled as part of Chromium's `chrome` target. The rest of
the Chromium checkout, `depot_tools`, and build outputs are local dependencies
and are intentionally ignored by Git.

## Major Pieces

- `MoriRoot.swift` owns the SwiftUI root, shortcut entry points, and the shared
  app model exposed to the Chromium bridge.
- `RootView.swift`, `TopChrome.swift`, `Toolbar.swift`, `Sidebar.swift`,
  `LauncherOverlay.swift`, and the panel views implement Mori's browser chrome.
- `BrowserStore.swift`, `BrowserTab.swift`, `TabFolder.swift`,
  `ExtensionStore.swift`, `DownloadStore.swift`, `HistoryStore.swift`, and
  `BookmarkStore.swift` are the Swift-side state model.
- `ShortcutRegistry.swift` is the single shortcut routing layer used by both
  AppKit event monitoring and Chromium web-view key pre-handling.
- `mori_chrome_bridge.mm` connects SwiftUI/AppKit chrome to Chromium browser
  commands, tabs, windows, menus, downloads, and extension state.
- `mori_browser_window.mm` and `mori_browser_window.h` hook Mori into the
  Chromium browser window lifecycle and web-focused keyboard handling.
- `mori_chrome_extensions.mm` integrates with Chromium's extension APIs. Mori
  now relies on Chromium's real extension system, not the old CEF compatibility
  bridge.
- `mori_permission_prompt.mm`, `MoriBrowserView.h`, `MoriPrivacy.h`, and
  `mori_bridge.h` are the remaining Objective-C++ integration surface between
  Chromium and the SwiftUI shell.

## What Changed From CEF

The old repository layout contained a separate macOS application under
`Sources/`, helper executables, CEF framework embedding scripts, and custom CEF
extension/runtime glue. That model is gone.

Mori now runs inside the Chromium app bundle produced by the ungoogled-chromium
build. Browser primitives such as tabs, navigation, extension pages, downloads,
permissions, and renderer input are owned by Chromium. Mori's code is the custom
native chrome and bridge layer around those primitives.

## Local Layout

```text
BUILDING.md
  Detailed local build, package, run, and logging instructions.

ungoogled-chromium-macos/build/src/chrome/browser/ui/mori/
  Mori's tracked Swift, Objective-C++, headers, icons, and glyphs.

ungoogled-chromium-macos/build/src/out/Default/Chromium.app
  Local Chromium build output. Not tracked.

/Users/choki/Downloads/MoriBrowser.app
  Packaged app used for manual testing. Not tracked.
```

## Build And Package

Use the full commands in `BUILDING.md`. The short version is:

```sh
cd /Users/choki/Developer/chromium-mori/ungoogled-chromium-macos/build/src
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
export PATH="$HOME/Developer/chromium-mori/binshims:$HOME/Developer/chromium-mori/depot_tools:$PATH"
ninja -j 16 -l 24 -C out/Default chrome

if [ -e "$HOME/Downloads/MoriBrowser.app" ]; then
  trash "$HOME/Downloads/MoriBrowser.app"
fi
/usr/bin/ditto "out/Default/Chromium.app" "$HOME/Downloads/MoriBrowser.app"
```

Use `trash`, not `rm -rf`, when replacing app bundles or build directories.

## Run

```sh
/usr/bin/open -n "$HOME/Downloads/MoriBrowser.app"
```

For a clean profile:

```sh
profile="$HOME/Library/Application Support/MoriBrowserTestProfile"
mkdir -p "$profile"
/usr/bin/open -n "$HOME/Downloads/MoriBrowser.app" --args \
  --user-data-dir="$profile" \
  --no-first-run
```

## Debugging Notes

Current keyboard-shortcut debugging builds emit `MORI-KEY` log lines from the
shortcut registry, the Swift store, Chromium web-view pre-handling, and the
AppKit event monitor. See `BUILDING.md` for commands to launch with stderr
logging and verify that an instrumented packaged framework was copied into
`MoriBrowser.app`.
