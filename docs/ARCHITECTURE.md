# Mori architecture

## Desktop: Chromium Views

The main desktop implementation compiles Mori's C++ interface into
ungoogled-chromium's browser. `linux/mori/` is shared by Windows, Linux, and
macOS despite its directory name.

```text
linux/mori/               Mori sidebar, commands, folders, settings, themes,
                          library, imports, PiP integration, and assistant
linux/patches/            Chromium build hooks and shared browser adaptations
windows/patches/          Windows-specific compatibility and window integration
Chromium + ungoogled      tabs/WebContents, renderer, network, extensions,
                          profiles, permissions, downloads, media, and DevTools
```

### Ownership and state

Chromium owns browser windows, `TabStripModel`, tab interfaces, WebContents,
sessions, and browser services. Mori uses those objects through Views and
browser observers rather than embedding a second web engine.

- `mori_toolbar_layout`, `mori_sidebar`, and `mori_tab_list` lay out the sidebar,
  toolbar, and page surfaces. The region's public tab-strip accessor avoids
  depending on Chromium's internal child-view hierarchy.
- `mori_command_bar`, `mori_commands`, and `mori_command_ids` route shortcuts,
  navigation, autocomplete, and named actions.
- `mori_folders` and the folder session/tree helpers persist folder membership
  and integrate tab close, unload, restore, and drag-and-drop behavior. An
  explicit removal is distinct from unloading a persistent tab's page.
- `mori_settings`, `mori_themes`, and the colour mixer own Mori preferences and
  interface styling. Native browser services continue to enforce permissions.
- `mori_picture_in_picture` observes pages and tabs, installs the video controls,
  and uses Chromium's native video PiP API. Preferences control hover controls,
  automatic entry, and returning automatic players to their source tab.
- The assistant uses a local `codex app-server` over stdio. The assistant client
  and browser tools are in `mori_codex` and `mori_assistant*`.

### Build boundary

First-party source stays in this repository. Host scripts link `linux/mori`
into a separate Chromium checkout and apply the numbered patches. Chromium
sources, downloaded tools, profiles, and build outputs remain local.

`windows/chromium.json` pins the shared patch baseline and Windows source/base
revisions. Windows setup records patch hashes and rejects changed applied
patches. Its build/package scripts lock the checkout, wait for older Ninja
processes, and serialize writers through compilation and installer creation.
Packaging verifies engine version, product names, and the original Mori icon.

Windows uses `windows/`; the Linux/macOS wrappers use matching upstream
platform packaging checkouts. Host verification status is in `BUILDING.md`.
macOS uses the shared Views interface plus a small Objective-C++ event adapter.
The retired desktop SwiftUI bridge and store are no longer part of the runtime.

## iOS

`ios/MoriBrowser` is a separate SwiftUI application around WKWebView. Its store,
workspace/session model, and interface are implemented inside the iOS project.
It does not link the desktop Views implementation.

## Historical design

The earlier migration rationale is retained in
[the archived decision record](archive/DECISIONS-linux-chromium.md). References
there to the previous desktop SwiftUI bridge describe historical work.
