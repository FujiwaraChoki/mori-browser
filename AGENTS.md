# Working in this repo

The repo holds two implementations of Mori:

- **Mori 3** (Rust, Linux first): the Cargo workspace in `crates/` - GPUI chrome
  (`crates/mori`), product logic (`crates/mori-core`), and Chromium via CEF
  (`crates/mori-engine`).
- **Mori 2** (macOS): a SwiftUI/AppKit browser chrome embedded into
  ungoogled-chromium. Its first-party source lives in
  `ungoogled-chromium-macos/build/src/chrome/browser/ui/mori`; the rest of the
  Chromium checkout is a local build dependency.

## Required

- Never use `rm -rf`.
- Use `trash path` when a file, directory, or app bundle needs to be removed.
- If browser automation is needed, use the Playwright CLI.
- Do not use the Codex Chrome extension for browser testing.

## Mori 3 (Rust)

- Build with `cargo build` / `cargo run --release`; CEF downloads into `.cef/`
  on the first build. Never commit `.cef/`, `target/`, or `dist/`.
- Put product rules in `mori-core` with unit tests; keep `mori-engine` a thin
  layer over CEF; UI lives in `crates/mori/src/ui`.
- Run `cargo test -p mori-core -p mori-engine -p mori` and
  `cargo clippy --workspace` before handing work back.
- On Linux there is no `trash` command; use `gio trash <path>`.
- Test with an isolated profile: `MORI_DATA_DIR=/tmp/mori-test target/debug/mori`.

## Build (Mori 2)

See `BUILDING.md` for the canonical local build, package, run, and logging
commands. The packaged app used for manual testing is:

```text
$HOME/Downloads/MoriBrowser.app
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
