# Working in this repo

Mori is now a SwiftUI/AppKit browser chrome embedded into
ungoogled-chromium. The first-party source lives in
`ungoogled-chromium-macos/build/src/chrome/browser/ui/mori`; the rest of the
Chromium checkout is a local build dependency.

## Required

- Never use `rm -rf`.
- Use `trash path` when a file, directory, or app bundle needs to be removed.
- If browser automation is needed, use the Playwright CLI.
- Do not use the Codex Chrome extension for browser testing.

## Build

See `BUILDING.md` for the canonical local build, package, run, and logging
commands. The packaged app used for manual testing is:

```text
/Users/choki/Downloads/MoriBrowser.app
```

## Architecture Notes

- SwiftUI state and views are in `*.swift` files under the Mori overlay.
- Objective-C++ Chromium integration lives in `mori_chrome_bridge.mm`,
  `mori_browser_window.mm`, `mori_chrome_extensions.mm`, and related headers.
- Chromium owns browser primitives such as tabs, navigation, renderers,
  extensions, downloads, permissions, and text input.
- Mori owns the native chrome, sidebar, launcher, shortcut registry, app model,
  and bridge layer around Chromium.

## Git Hygiene

- The full Chromium checkout, `depot_tools`, and build products are local and
  should not be committed.
- The Mori overlay path is intentionally tracked even though it sits inside a
  Chromium checkout.
- If the local ungoogled-chromium ignore rules hide Mori overlay files, stage
  them explicitly with `git add -f`.
