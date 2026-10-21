# Mori on Linux: build on Chromium itself, not CEF

Status: decided 2026-10-17. Step 1 (stock build) done the same day; step 2
(Mori window spike) done 2026-10-18; step 3 (porting features) in progress.

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
- `0004-mori-content-card-layout` (now `0004-mori-window-layout`): with tabs
  in the sidebar, the page is an inset card with rounded corners.
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

## Step 3 progress

Step 2's window still looked like Chrome with vertical tabs: a toolbar across
the top, Chrome's tab rows, and Chrome's sidebar header. The window now
follows Arc and Zen's single-toolbar layout.

- **Toolbar in the sidebar** (`mori_toolbar_layout.cc`, `0004`). Chromium's
  own toolbar moves to the top of the expanded sidebar and lays out in two
  rows. Row one has the window buttons, back, forward and reload, and
  downloads, extensions and the menu at the end. Row two is the address
  field. Moving the real toolbar, rather than drawing a new one, keeps every
  bubble Chromium anchors to it working: site info, permissions, extension
  popups and downloads.
  - The toolbar keeps a `FlexLayout` subclass, because `ToolbarView` holds a
    pointer to its layout. It switches to two rows while the browser layout
    sets `kSidebarToolbarLeadingInsetKey`.
  - The decision is made before layout starts, so every measurement agrees.
  - The toolbar's one-row height cap is skipped in sidebar mode.
  - Its minimum height is one row, so the frame sizes the window buttons to
    the first row.
  - When the sidebar is collapsed (Ctrl+Shift+L or Ctrl+Alt+C), the toolbar
    goes back above the page.
- **Window buttons on the left** (`0004`). All window buttons sit at the
  start of the sidebar's first row, like Arc's traffic lights. The frame
  uses the same order for placing buttons and for the exclusion areas it
  reports to the layout. With the two out of step, the layout saw a trailing
  exclusion as wide as the window.
- **No Chrome sidebar header.** The tab strip's own header row (collapse,
  tab groups, tab search) is hidden in sidebar-toolbar mode.
- **The page is a card on all four sides.**
- **Flat tab rows.** Only the active and hovered tabs get a fill.
- **The address field shows the site.** ungoogled-chromium turns URL
  elision off; Mori turns it back on.
- **The new tab page uses the card colour.**
- **Favorites are pinned tabs**, which Chromium's vertical tab strip already
  draws as a grid of tiles above the tabs. They are shared by every space,
  as in Arc.
- **Spaces are tab groups** (`mori_spaces.cc`, `0006`). Chromium 155's
  `kTabGroupsFocusing`, enabled by default in Mori, "focuses" a group: the
  sidebar shows that group's tabs and the pinned tabs, tinted in the group's
  colour.
  - A space bar at the bottom of the sidebar shows a dot per space and a
    "+" for a new space.
  - Ctrl+Alt+←/→ (and Chromium's Alt+Shift+X/Z) switch spaces, wrapping
    around.
  - Every tab that isn't a favorite is in a space, and a space is always on
    screen, as in Arc. A window without spaces gets a "Personal" space for
    its tabs. Loose tabs, for example from session restore, join the space
    on screen. If Chromium unfocuses (say the focused space's last tab
    closes), the active tab's space is shown.
  - Spaces are only created and named once the saved-tab-groups service has
    initialised. That service applies its stored copy of each group when it
    starts, which reset a "Personal" space made earlier to an unnamed grey
    group.
  - New tabs join the space on screen: from the command bar, links,
    extensions and the new-tab button. `TabStripModel::AddTab` gives a new
    unpinned tab the focused group, because Chromium otherwise drops focus
    as soon as such a tab is inserted.
  - Switching to an open tab from the command bar shows its space.
  - Spaces, and which one is focused, come back after a restart.
- **Session restore by default**, as in Arc.
- **Command bar fix.** For address-like input, the typed address ranks
  first, ahead of searches and open tabs.
- **Commands in the command bar** (`mori_commands.cc`). Typing part of a
  command's name lists it above the suggestions:
  - spaces: new, next, previous;
  - favorites: add or remove the current tab;
  - toggle sidebar, split view, duplicate tab, copy URL, close tab;
  - new window, new private window;
  - history, downloads, bookmarks, extensions, settings;
  - clear browsing data, developer tools, print.

  Every word typed has to start a word of the command's name or keywords.
- **Test fixture** (`linux/harness/fixture-ext`, `FIXTURE=1 run.sh`). An
  extension that pins two tabs and makes "Work" and "Leisure" groups on
  first run. Ctrl+Shift+U opens a tab the way an extension or link would.

## Step 4: Mori 3's look and features

The window now matches Mori 3 side by side at 1.5× (sidebar metrics, Lucide
icons, the launcher, menus), and most of Mori 3's features are ported. Each
lives in `linux/mori/` and reaches Chromium through a small patch.

- **Arc's Ctrl+S** (`mori_sidebar.cc`): the sidebar hides; at the window's
  left edge it floats back over the page, with rounded corners and a shadow.
- **Spaces outlive their last tab.** Closing or moving a space's last tab
  brings the space back with a new tab; "Delete Space" removes one on
  purpose. Chromium's "delete group?" prompts are off (`0005`).
- **Space editor** (`mori_space_editor.cc`, `mori_space_themes.cc`, `0015`):
  name, symbol (a Lucide icon or an emoji) and one of Mori 3's gradients. The
  gradient washes over the sidebar, toolbar and margins through a hook in
  `ThemedBackground`. The sidebar's surfaces are translucent, as in Mori 3,
  so the wash shows through. Themes are kept in "Mori Spaces.json", keyed by
  saved tab group.
- **Clear and auto-archive.** The divider's "Clear" closes a space's tabs.
  Space tabs unused for 24 hours close; they stay in History and Reopen
  Closed Tab.
- **Command bar parity**: switch to or move a tab to a space, move a tab to
  a new window, edit or delete a space, reopen a closed tab, find, capture,
  peek the clipboard link, Boosts, routing rules, import, the assistant. Each
  row shows the command's shortcut.
- **Ctrl+Tab** goes through the space's recent tabs, with thumbnails
  (`mori_tab_switcher.cc`). **Ctrl+/** opens a searchable shortcut
  cheatsheet built from the live key bindings.
- **Peek** (`mori_peek.cc`, `0010`): Alt+click opens a link in a card over
  the page, with Close, Open as Tab and Split View. Blink sends Alt+click
  through as a picture-in-picture disposition, which the browser's
  `OpenURLFromTab` turns into Peek. Ctrl+Shift+O peeks the link on the
  clipboard.
- **Capture** (Ctrl+Shift+X) uses Chromium's screenshot flow. `0011` fixes
  its selection layer, which was offset by the sidebar's width.
- **Bubbles open into the window** (`0012`). Wayland gives Chromium no
  screen position, so it never moved bubbles back on screen. Ones anchored
  to the sidebar's toolbar opened past the window's left edge; they now
  mirror against the window.
- **Tab names** (`mori_tab_names.cc`, `0013`): double-click a tab, or "Rename
  Tab". Names go into the session's per-tab extra data, which the patch also
  writes whenever Chromium rewrites the session file.
- **Boosts** (`mori_boosts.cc`, `0014`): per-site CSS, JavaScript and zapped
  elements, applied from an isolated world as pages load. They are kept in
  "Mori Boosts.json".
- **Links from other apps** (`0016`). Air Traffic Control rules
  (`mori_routes.cc`, "Mori Routes.json") send a site's links to a space.
  Other links open in Little Mori (`mori_little.cc`): a 900×640 popup window
  with back, the address, "Open in <space>" and close. The command bar
  switches between Little Mori and the current space.
- **Folders** (`mori_folders.cc`, `0028`, hooks in `0007` and `0013`).
  Zen's folders inside a space, with Zen's folder icon (its two plates,
  tilting open). Tab groups are spaces and can't nest, so a folder is
  Mori's: each tab keeps its folder (id, name, icon, open) in its session
  data, and a folder is the run of a space's tabs that share one. A tab
  moved between two of a folder's tabs joins it; one moved away leaves.
  A closed folder still shows its active tab, and its icon shows dots.
- **Smooth corners** (`mori_corners.cc`, `0029`). Zen draws every rounded
  rect with `corner-shape: superellipse(1.3)` on radii 1.3 times as long;
  Mori does the same in Canvas::DrawRoundRect, rounded backgrounds and
  painters, highlights, focus rings, menu items and tab rows. Corners
  clipped by compositor layers (the page card, the floating sidebar) and
  bubble outlines stay circular.
- **Liquid Glass** (`mori_glass.cc`; hooks in `0002`, `0004`, `0007`), as in
  Mori 3: the toolbar's two button groups stand in raised glass capsules (a
  view behind the buttons, `mori_toolbar_layout.cc`); the address field and
  the favorites are set into the glass with a light rim; the active tab is
  a lens, brighter at the top, with a rim and a line of light along its top
  edge. The page card's corners are 10px. The window itself stays opaque:
  it would need a translucent browser widget, and Hyprland's blur.
- **The sidebar as Mori 3 draws it** (`mori_spaces.cc`, `0004`, `0005`,
  `0031`): no space name or "Clear" between the favorites and the tabs, only
  a hairline under the favorites and "Add Tab". Clear is "Archive Unpinned
  Tabs" in the space's menu and the command bar, as in Mori 3. Favorites are
  44px tiles, up to four a row, balanced so no row has a hole and each row
  fills the sidebar. The address field shows the site alone (Chromium's
  "Always show full URLs" still shows the rest). The space bar's dots close
  up to keep "+" in the bar, and the library button is Lucide's
  library-big.
- **Focus rings with smooth corners** (`0029`). A focus ring takes the round
  rect its view's highlight was made from: the smooth highlight path isn't
  one, and FocusRing CHECKed that it was (the crash on the Web Store's "Add
  to Mori", whose dialog focuses a button).
- **Zen's tab animations** (`0030`). A sidebar tab fades in and grows from
  0.95 as it opens (120ms) and fades and shrinks back as it closes (100ms),
  easing out, while the rows around it move on the same timing. Chromium's
  own were 200ms, ease-in-out, without the scale.
- **Alt+1–9 picks a tab on Linux** (Chromium's Linux keys, beside Ctrl+1–9),
  and the spaces moved to Ctrl+Alt+1–9. Windows keeps Alt+1–9 for spaces
  (Ctrl+Alt is AltGr there).
- **Import** (`mori_import.cc`, `0017`). Zen's workspaces and essentials, a
  Firefox session, and Arc's sidebar file become spaces and favorites. Tabs
  arrive unloaded and fetch their icons. Chromium's importer finds Zen
  profiles too, for bookmarks, history and passwords.
- **Assistant** (`mori_assistant*.cc`, `mori_codex.cc`): Mori 3's Codex
  panel, docked beside the page by the browser layout (Ctrl+Shift+K or the
  sparkles button).
  - Talks to `codex app-server` over stdio, with Mori 3's browser tools.
  - Page scripts run in Mori's isolated world. Because Chromium's public
    isolated-world call doesn't await promises, each script parks its result
    in the world and the browser polls for it.
  - Every page read or browser change waits for Allow.
- **Toasts** (`mori_toast.cc`): Zen's pill for "Opened in <space>", rule
  changes and import results.

Chromium surfaces redrawn as Mori 3's (patches 0018-0022):
- Right-click menus on tabs, the sidebar's empty space and pages: Mori's
  items, Lucide icons and Title Case. The page menu keeps Chromium's own
  entries (spelling, extensions, autofill) and adds Peek, spaces, bookmark,
  copy link and capture.
- The library popover (History, Bookmarks, Archive of closed tabs) for the
  library button and Ctrl+Y.
- A settings sheet over the page for Ctrl+, bound to Chromium's
  preferences, with a way through to Chromium's full settings page.
- The new tab page (logo and search field), the empty command bar (recent
  tabs and history), the find bar at the page's top right, overlay
  scrollbars, Ctrl+D as a toast with a yellow star, a grey placeholder
  favicon, and Mori's blue in Chromium's bubbles and dialogs.

Still to port from Mori 3:
- Per-space pinned tabs. Chromium's tab groups have none; it needs Mori's
  own bookkeeping per saved group.
- Nested folders, dragging a folder, and dragging a tab onto a folder's
  header (a tab dropped between two of a folder's tabs joins it).
- Downloads as Mori 3's popover (Chromium's bubble shows now).
- Content blocking (uBlock Origin, once MV2 support is checked).
- Branding: the icon and a .desktop file. The name
  is done: patch 0023 (made by linux/tools/rebrand_strings.py) calls the
  browser Mori in Chromium's strings, and run.sh sets the "mori" app ID.

## macOS

The Views code is shared with macOS, and the macOS paths are written but
not yet built or run (that needs a Mac with Xcode and ungoogled-chromium's
macOS tree; this machine can't cross-build it). What differs:

- Shortcuts: macOS takes them from the menu bar and
  global_keyboard_shortcuts_mac.mm, not from Mori's accelerator list
  (mori_command_ids.cc skips it there). Patch 0025 adds Mori's with Command
  for Control and Arc's keys where macOS needs others: Cmd+S hides the
  sidebar, Ctrl+1-9 picks a space, Cmd+Option+Left/Right switches spaces,
  Cmd+Option+Up/Down the tab, Ctrl+Tab is the recent-tabs switcher, and
  Cmd+Shift+D duplicates. Save Page As, the bookmarks bar and Bookmark All
  Tabs lose their shortcuts, and the View menu's vertical-tabs item reads
  "Toggle Sidebar".
- Window frame: the traffic lights sit at the left of the sidebar
  toolbar's first row (the layout leaves the frame's leading exclusion
  free, and macOS centres them in the toolbar's minimum height). They hide
  with the sidebar and come back when it peeks out, as in Arc
  (mori_window_events_mac.mm).
- Input: Mori watches a window's events through mori_window_events.h,
  which uses an application monitor on Aura and a window monitor on macOS,
  where there is no Aura.
- Menus are macOS's native ones, with Mori's items and icons.
- Import reads Zen, Firefox and Arc from ~/Library/Application Support.
- The profile is ~/Library/Application Support/Mori; the menu bar and Dock
  say Mori (the bundle's display name), though the bundle's files keep
  Chromium's names until a full rename can be tested on a Mac.
- Codex is also looked for in /opt/homebrew/bin, and its directory goes on
  its PATH, since apps opened from the Dock get a minimal one.

## Windows

Windows Chromium uses Aura, as Linux does, so the Views code and Mori's
input handling carry over unchanged. Like macOS, it is written but not yet
built or run (that needs Windows and Visual Studio). What differs:

- Window frame: Windows draws its window buttons at the top right, where
  the page card would be. When the frame reports them (its trailing
  exclusion), the card starts below them, leaving a strip as in Zen, and
  BrowserView answers the strip as window frame, so it drags the window
  (patches 0004 and 0020). Linux, with no buttons, and macOS, with them at
  the left, keep the card at the top.
- Shortcuts are Linux's (Ctrl), except Ctrl+Alt+S and Ctrl+Alt+R: on
  Windows Ctrl+Alt is AltGr, which types characters on many keyboards.
- Codex runs through pipes and a job object (closing the job ends codex
  and the node it starts); npm's codex.cmd runs through cmd.exe, and Mori
  also looks in %APPDATA%\npm, WinGet's and Scoop's folders. One reader
  and writer serve all three systems, on base::File.
- The profile is %LOCALAPPDATA%\Mori\User Data and policies come from
  SOFTWARE\Policies\Mori (patch 0026: the install's product name).
- Import reads Zen and Firefox from %APPDATA% and Arc from its packaged
  app's folder under %LOCALAPPDATA%\Packages.
- Overlay scrollbars and reopening the last session are on, as on Linux.
- The executable and installer keep Chromium's names (chrome.exe, the
  ChromiumHTM ProgID) until a full rename can be tested on Windows.

## Chrome Web Store

Extensions install from the Chrome Web Store, as in Arc,
and update from it (patch 0027). ungoogled-chromium cuts
the store off in three ways, which Mori undoes:

- Domain substitution renames the store's addresses
  (chromewebstore.9oo91e.qjz9zk), so the real store isn't recognised and
  gets no webstorePrivate API: "Add to Chrome" stays grey. The store,
  install and update addresses are the real ones again, and the store gets
  its dedicated process back.
- The store installer's download and install steps are emptied: restored.
- Update checks never start, and extensions without an update URL get
  none: restored, so extensions from the store update from it.

Google's update pings (daystart) and OAuth on update requests stay off.

The store names the browser Chrome and asks Chromium-based browsers to
switch. A tab helper (`mori_web_store.cc`) runs in an isolated world on the
store: "Chrome" becomes "Mori" in its buttons ("Add to Mori", in any
language), and its switch-to-Chrome popup (which links to Chrome's
download) and banner (whose button is logged as 276336) are hidden.

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
