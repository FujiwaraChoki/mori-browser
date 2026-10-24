# Mori Browser

Mori is a Chromium-based desktop browser with a compact sidebar, persistent
favorites and folders, a command bar, themes, and an assistant beside the page.
The desktop interface is built with Chromium's Views toolkit in `linux/mori/`.
Windows, Linux, and macOS share that source and the patches in `linux/patches/`.

The current Windows build uses **Chromium 156.0.8078.47** with the matching
ungoogled-chromium base. Its exact revisions live in
[windows/chromium.json](windows/chromium.json).

## Implementations and platform status

| Implementation | Source | Status |
| --- | --- | --- |
| Desktop, Chromium Views | `linux/mori`, `linux/patches` | Main desktop implementation. Chromium 156 build and headless tests verified on Windows. |
| Windows build and installers | `windows` | Native per-user installer and visible setup wizard, with Mori branding and version checks. |
| Linux and macOS Views builds | `linux/*.sh`, `mac/*.sh` | Host build scripts retained; use a matching upstream Chromium/ungoogled checkout. The Chromium 156 port still needs validation on these hosts. |
| iOS app | `ios` | Independent SwiftUI/WKWebView application and Xcode project. |

Retired desktop implementations remain available in Git history. The working
source tree contains the Chromium Views desktop browser and the iOS app.

## Desktop features

- Persistent favorites and folders: closing a persistent tab unloads its page;
  explicit removal deletes the saved entry. Tabs can move between ordinary
  tabs, folders, and favorites through drag and drop.
- Command bar, recent-tab switcher, shortcut reference, context menus, and a
  sidebar menu with folder creation.
- Themes, rounded page surfaces, a library, archive, imports, Reader, page
  capture, Boosts, Peek, and Little Mori for external links.
- Chromium's browser engine, extensions, downloads, navigation, permissions,
  and DevTools, with Mori's interface and ungoogled-chromium patches.
- Configurable video Picture in Picture, including a hover control and optional
  automatic entry when switching tabs. H.264/AAC is enabled in the Windows build.
- Assistant integration through a local `codex app-server` process.

Desktop and iOS feature sets differ; the list above describes the desktop
implementation.

## Build Windows

See [BUILDING.md](BUILDING.md) for prerequisites and checkout preparation.
Once the pinned Chromium checkout is synced:

```powershell
python windows/build.py --jobs 12
python windows/package.py --jobs 12
python windows/build-wizard.py
```

Build products are written to the local Chromium checkout and `dist/`.
Build/package commands serialize access to a checkout so parallel tasks share
completed work without overlapping writes. Dependencies and generated binaries
are excluded from Git.

## Tests

```powershell
python -m unittest discover -s windows/tests -p "test_*.py" -v
npm ci --prefix windows
npm --prefix windows test
```

The Playwright CLI suites launch the built Mori executable in headless mode,
with the Chromium sandbox enabled and isolated temporary profiles. They cover
the pinned engine version, PiP controls and preferences, and actual H.264/AAC
URL and MediaSource playback. `MORI_TEST_BROWSER` selects a separate runtime
copy if the build output needs to remain available to other builds.

## Documentation

- [Build, package, run, and test](BUILDING.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Source layout](docs/SOURCE_LAYOUT.md)
- [Shared desktop behavior and Linux tooling](linux/README.md)
- [Historical Chromium migration record](docs/archive/DECISIONS-linux-chromium.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)
