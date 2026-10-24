# Working in this repo

## Implementations

- **Desktop Chromium Views** is the main desktop implementation. Shared C++
  source lives in `linux/mori/`; `linux/patches/` hooks it into Chromium.
  `windows/` supplies the Windows checkout, compatibility patches, packaging,
  and tests. `linux/*.sh` and `mac/*.sh` are the other host build entry points.
- **iOS** is the independent SwiftUI/WKWebView app in `ios/`.

The desktop SwiftUI and Rust/CEF prototype implementations are retired. Keep
shared artwork in `mac/assets/` and native macOS integration in `linux/mori/*.mm`.

## Required

- Never use `rm -rf`.
- Use `trash path` when a file, directory, or app bundle needs to be removed.
  On Linux use `gio trash`; on Windows use the Recycle Bin equivalent.
- If browser automation is needed, use the Playwright CLI.
- Do not use the Codex Chrome extension for browser testing.

## Desktop Views

- Read `BUILDING.md` for platform status, build, package, run, and test commands.
- The shared patch baseline and Windows source/base revisions are pinned in
  `windows/chromium.json`. Use matching Chromium and ungoogled sources.
- Keep first-party desktop behavior in `linux/mori`; keep changes to Chromium
  in numbered patches. Windows compatibility patches belong in `windows/patches`.
- Sync dependencies before applying patches. Applied patch hashes are recorded;
  do not silently rewrite a patch that is already applied to a build checkout.
- Use the build/package scripts to serialize checkout access. Avoid concurrent
  direct Ninja invocations in the same output directory.
- Use isolated profiles for tests. Do not reuse the installed browser's profile.
- For Windows changes, run the relevant Python tooling and headless Playwright
  suites. Verify installer version, Mori product names, and original icon assets.
- Linux and macOS changes need checks on their target hosts; a Windows build
  does not validate those platforms.

## Linux input harness

`linux/harness/vptr/` is a standalone Rust virtual-pointer utility for the
headless Wayland harness. It has its own Cargo workspace and remains in use;
it is not the retired browser prototype. Build it on Linux when using that
harness.

## Git hygiene

- Full Chromium checkouts, `depot_tools`, downloaded toolchains, profiles, and
  generated build/install outputs stay local and must not be committed.
- Commit shared source, numbered patches, build scripts, tests, docs, and the
  original Mori branding assets.
- Preserve the independent iOS application and Linux input harness unless
  explicitly asked to remove them.
