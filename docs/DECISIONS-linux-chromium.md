# Mori on Linux: build on Chromium itself, not CEF

Status: decided 2026-10-17. Step 1 (stock build) done the same day; step 2
(Mori window spike) done 2026-10-18.

## Context

Mori 3 (`crates/`) is a Rust GPUI interface around Chromium embedded through
CEF in windowless (offscreen) mode. It works, but CEF limits what Mori can be:

- Chrome extensions don't work. CEF leaves out Chrome's browser layer, and
  windowless browsers can't host extension UI.
- CEF only ships even-numbered Chromium milestones. Mori 3 is on CEF 154
  (Chromium 154.0.8037.98) while Chrome stable is 155.0.8059.84.
- Offscreen rendering needs workarounds that a real browser window doesn't:
  - the clipboard is bridged through page scripts, because the x11 clipboard
    breaks under CEF's external message pump;
  - `--ozone-platform=headless` makes Chromium deny geolocation and
    notification prompts;
  - every frame is copied into GPUI;
  - nothing can be dragged out of a page.
- Things Chrome already has had to be rebuilt by hand in Mori 3: the content
  blocker, the password manager, picture-in-picture, permission prompts,
  printing, file choosers, DevTools hosting and downloads.

Arc and Mori 2 (macOS) don't use CEF. They compile their own interface into
a Chromium build.

## Decisions

1. **Engine: ungoogled-chromium, built from source on Linux.** Same base as
   Mori 2. It follows Chrome stable (155.0.8059.84 on 2026-10-17). Mori's code
   is compiled into Chromium's `chrome` target, as Mori 2 does on macOS.

2. **Interface: C++ with Chromium's Views toolkit.** Mori's window is a custom
   browser window with its own Views-based sidebar, command bar and panels,
   styled to match Zen and Arc. Extension popups, permission bubbles and other
   Chromium UI stay Chromium's own.
   - Rejected: an HTML/CSS/TypeScript interface (WebUI, as Vivaldi does, and
     Chromium's experimental `chrome/browser/ui/webui_browser` behind
     `kWebium`). It is faster to iterate on, but we chose the native toolkit.
   - Rejected: keeping GPUI. Chromium owns the window on Linux, and embedding
     a second GPU renderer and input stack into it is a research project.

3. **Chromium owns browser primitives; Mori owns the interface and product
   rules.** Tabs, navigation, renderers, extensions, downloads, permissions,
   passwords, printing, PiP and DevTools come from Chromium. Mori owns the
   window, sidebar, spaces, favorites/pinned/Today model, command bar,
   shortcuts and theming. This is the same split as Mori 2.

4. **Reuse what still fits:**
   - `mori-core` holds Mori 3's product rules in plain Rust with tests: the
     workspace model, launcher ranking, URL-vs-search rules, the theme engine
     and import. Link it into Chromium through Chromium's Rust support (GN
     Rust targets plus cxx) where that pays off. Its dependencies have to be
     available in Chromium's `third_party/rust`; port to C++ where they aren't.
   - Mori 2's bridge is the reference for hooking into Chromium:
     `mori_browser_window.mm`, `mori_chrome_bridge.mm`,
     `mori_chrome_extensions.mm` and `mori_permission_prompt.mm`, about 4,400
     lines of Objective-C++.
   - Mori 3's GPUI interface (`crates/mori`) and CEF layer (`crates/mori-engine`)
     are not carried over. Neither are its hand-built replacements for Chrome
     features.

5. **Content blocking moves to an extension** (e.g. uBlock Origin). Still to
   check: how far ungoogled-chromium supports the MV2 extension format that the
   full uBlock Origin needs.

## Build environment

- Machine: AMD Ryzen 9 9950X3D (32 threads), 60 GB RAM, about 1.5 TB free on
  `/home` (btrfs).
- Chromium recommends about 64 GB of RAM. Use a component build for
  development and limit parallel link jobs.
- Estimates, not yet measured: 40–50 GB of source, 60–100 GB of build output,
  1–2 hours for the first full build, minutes for incremental builds of Mori
  code.

## Plan

1. Build stock ungoogled-chromium 155 on this machine to prove the toolchain.
   **Done 2026-10-17** (see "Step 1 setup" and "Step 1 results" below).
2. Spike a Mori window: a custom browser window with a Views sidebar and
   command bar around real Chromium tabs, with an extension installed and
   working. **Done 2026-10-18** (see "Step 2 results").
3. Port features from Mori 3's model and the Mori 2 bridge.

## Step 1 setup

- Mori's own files are tracked in `linux/` (see `linux/README.md`): build
  scripts, patches, the Views sources in `linux/mori/` and the test harness.
- Checkout: `ungoogled-chromium-linux/` in this worktree (git-ignored). It is
  a clone of `ungoogled-chromium-portablelinux` (with the `ungoogled-chromium`
  submodule) at 155.0.8059.84-1. Sources are in
  `ungoogled-chromium-linux/build/src`, build output in `out/Default`.
- `linux/fetch.sh` downloads the source tarball and applies pruning, patches
  and domain substitution, using the portable repo's `scripts/shared.sh`.
- `linux/build.sh` builds natively on Arch, without the portable repo's Debian
  Docker image:
  - It downloads Chromium's own clang, Rust and Debian bullseye sysroot.
  - Tools the portable patches expect in `/usr/bin` are installed locally
    under `build/host-tools`: esbuild 0.28.2 and TypeScript 7.0.2.
  - It writes development GN args: not official, component build, no
    symbols, and `use_mold=false` (mold is the default for development
    builds but isn't in the tarball).
- System packages needed: `gperf`, `ninja`, `go`, `clang-format`, and node
  (from mise).
- The build runs as a user systemd unit at idle priority:
  `systemd-run --user -p Nice=19 -p IOSchedulingClass=idle -p CPUWeight=10
  -p MemoryHigh=24G -p MemoryMax=30G`, with `JOBS=12` (ninja `-j12 -l12`).

## Step 1 results

- `out/Default/chrome` reports Chromium 155.0.8059.84 (Developer Build,
  ungoogled-chromium), V8 15.5.35.25. It runs natively on Wayland
  (`--ozone-platform=wayland`).
- An unpacked MV3 extension loads: it is listed in `chrome://extensions`, its
  content script runs, and its toolbar action is present.
- The first build took about 2.5 hours at idle priority with `-j12` while the
  machine was in use, and peaked well under the 30 GB cap. Sources plus build
  output take 18 GB, much less than estimated because there are no symbols.
- One fix was needed: ungoogled-chromium removes
  `managed_profile_creation_controller.cc`, but a test-only call to it in
  `profile_management_disclaimer_service.cc` survives in non-official
  builds. `mori-patches/0001-drop-unreachable-managed-profile-test-path.patch`
  removes the call. `linux/build.sh` applies `linux/patches/*.patch`.
- "Your browser is managed" appears because this machine has
  `/etc/chromium/policies/managed/color.json` (Omarchy theming). A Mori-branded
  build reads its own policy directory.
- Test harness (headless sway): `linux/harness/`.

## Step 2 results

Mori's code sits in `linux/mori/` (linked into the tree as
`chrome/browser/ui/views/mori/`). Chromium files get small patches that call
into it, so a Chromium upgrade means rebasing a few hunks, not a fork:

- `0002-mori-hooks`: Ctrl+T and Ctrl+L open Mori's command bar; Mori's
  sources and colour mixer join the build.
- `0003-mori-profile-and-policy-dirs`: the profile is `~/.config/mori`, and
  policies come from `/etc/mori/policies`, not Chromium's. This also stops
  Omarchy's Chromium theme policy from marking Mori as managed.
- `0004-mori-content-card-layout`: with tabs in the sidebar, the page is an
  inset card with rounded corners.
- `0005-mori-default-prefs`: tabs in the sidebar and no bookmarks bar by
  default.
- Also in `0005`: the default search
  engine is DuckDuckGo, because ungoogled-chromium replaces Google with
  "No Search".

What the spike shows:

- **Sidebar.** Chromium 155 ships a vertical tab strip, so the sidebar's tab
  list is Chromium's own, with dragging, groups, pinning and context menus.
  Mori restyles it rather than replacing it. Spaces, favorites and Today are
  step 3 and will be Mori views around this list.
- **Command bar** (`mori_command_bar.cc`). A floating field over the page, in
  a child widget like Chromium's find bar. Results come from Chromium's
  autocomplete: what was typed, history, bookmarks, search and open tabs
  ("Switch to tab"). Ctrl+T opens a result in a new tab. Ctrl+L starts from
  the current URL and replaces the page. When the input looks like an
  address, the typed address ranks above open tabs.
- **Look** (`mori_color_mixer.cc`, `mori_layout.h`). Mori 3's Zen palette
  (`#131313` window in dark mode, `#E9E9E9` in light mode) is one surface
  for the sidebar and toolbar, with the page as a rounded card. Installed
  themes and the GTK theme are left alone.
- **Extensions.** An unpacked MV3 extension's content script runs, and its
  popup opens from the toolbar (`Ctrl+Shift+Y` in the test extension).
- **Rebuilds** after a change to Mori's code take one to three minutes at
  idle priority.

Notes for step 3:

- The command bar's widget must not be activatable (like the find bar).
  Otherwise the browser window loses activation when it opens, focus clears,
  and the bar closes at once.
- The bar stays open when the window loses focus and closes when focus moves
  elsewhere in the window, for example into the page.
- Product rules such as result ordering are still C++. They belong in
  mori-core once Chromium's Rust build is wired up.

## Risks

- Following Chromium means rebasing Mori's patches every four weeks.
- First builds are long, and the machine has slightly less RAM than
  Chromium recommends.
- Chromium's Wayland support has its own quirks.
- Views styling needs care to look like Zen and Arc rather than like Chrome.

## Open questions

- Where the Chromium checkout lives: inside this repo (git-ignored, like
  Mori 2's `ungoogled-chromium-macos/build/src`) or beside it.
- Whether Mori 2 (macOS) and the Linux build should share one C++ interface
  later.
- What happens to the Mori 3 Rust/CEF code once the Chromium build can replace
  it: keep it, archive it on a branch, or remove it.
