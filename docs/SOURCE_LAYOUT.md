# Source layout

## Main desktop implementation: Chromium Views

| Path | Responsibility |
| --- | --- |
| `linux/mori/` | Shared first-party desktop C++ and native macOS event adapter. |
| `linux/patches/` | Numbered patches connecting Mori to Chromium's browser and Views toolkit. |
| `windows/` | Checkout preparation, build/version pins, Windows patches, branding, installers, and tests. |
| `linux/*.sh`, `linux/harness/` | Linux Views build and isolated compositor harness. |
| `mac/*.sh`, `mac/assets/` | macOS Views build/package helpers and shared original branding artwork. |
| `docs/`, `BUILDING.md` | Architecture, platform status, source map, and build workflows. |

The main desktop feature files are:

- `mori_command_bar.*`, `mori_commands.*`, `mori_command_ids.*`: navigation,
  autocomplete, commands, and keyboard bindings.
- `mori_sidebar.*`, `mori_tab_list.*`, `mori_toolbar_layout.*`: window/sidebar
  layout, library/assistant entry points, and tab presentation.
- `mori_folders.*`, `mori_folder_tree.h`, `mori_folder_session.h`: persistent
  folders, tab membership, unload/removal behavior, and drag-and-drop targets.
- `mori_settings.*`, `mori_themes.*`, `mori_color_mixer.*`, `mori_typography.h`:
  preferences, colours, themes, and shared UI type size.
- `mori_menu.*`, `mori_app_menu.*`, `mori_library.*`, `mori_import*`: menus,
  history/bookmarks, and importing existing browser data.
- `mori_picture_in_picture.*`, `mori_picture_in_picture_script.h`: native video
  PiP and page controls.
- `mori_peek.*`, `mori_little.*`, `mori_boosts.*`: Peek, external-link windows,
  and per-site customizations.
- `mori_codex.*`, `mori_assistant*`: assistant connection, presentation, and tools.
- `mori_window_events_mac.mm`: native macOS event integration for Views.
- `tests/`: native Chromium import/passkey tests; `linux/tests/` has additional
  focused tests. Windows headless suites and tooling tests are in `windows/tests/`.

### Dependency and output boundaries

Full Chromium trees, upstream packaging checkouts, downloaded toolchains,
`depot_tools/`, `.cef/`, `target/`, and `dist/` are ignored local dependencies or
outputs. The Windows build normally uses the short external checkout path in
`windows/chromium.json`. Linux and macOS use ignored platform checkouts beside
these host script directories. No first-party source needs to be kept inside a
tracked Chromium dependency tree.

The retired desktop SwiftUI and Rust/CEF implementations have been removed
from the working tree.
Their historical source remains available in Git history. Shared app icons remain
in `mac/assets/` and are also used by Windows branding.

The standalone Rust helper in `linux/harness/vptr/` supplies virtual pointer
input to headless sway tests. Its Cargo workspace is independent of the browser.

## iOS

```text
ios/project.yml                 XcodeGen project definition
ios/MoriBrowser.xcodeproj/       checked-in Xcode project and scheme
ios/MoriBrowser/Model/           workspace, session, commands, records
ios/MoriBrowser/Store/           application state
ios/MoriBrowser/UI/              SwiftUI interface
ios/MoriBrowser/Web/             WKWebView session, reader, favicon helpers
ios/MoriBrowser/Theme/           theme tokens
ios/MoriBrowser/Assets.xcassets/  iOS app artwork
ios/MoriBrowserTests/            iOS tests
```

## Historical documentation

`docs/archive/DECISIONS-linux-chromium.md` records the original migration and
feature-porting work. Use `BUILDING.md` for current build commands and versions.
