# Building Mori Browser

The main desktop browser uses Chromium Views. Its shared source is
`linux/mori/`, with hooks in `linux/patches/` and host scripts in `windows/`,
`linux/`, and `mac/`. The retired desktop SwiftUI overlay is no longer used.
The iOS app is a separate WKWebView build.

For hosted checks and manual Windows/Linux builds on the configured PC,
see [CI and local build runners](docs/CI.md). macOS CI is deferred.

## Windows: Chromium Views


Build it with a full Chromium **156.0.8078.47** checkout and its dependencies
synced with `depot_tools`.

Prerequisites are Visual Studio 2026 C++ Build Tools (including MFC/ATL),
Windows SDK 10.0.28000.2270 or newer with SDK Debugging Tools, Git for Windows
(including GNU `patch`), and Python 3. Keep `depot_tools` at the repository root
and set `DEPOT_TOOLS_WIN_TOOLCHAIN=0`. The version, source revision, and matching
ungoogled release are pinned in
`windows/chromium.json`. The ungoogled patch repository lives in the checkout's
`ungoogled-base` directory, pinned to `156.0.8078.47-1`.

Use a short checkout path to avoid Windows path-length failures. On this
machine the upgraded checkout lives at `C:\mori-build-156`; the working
155 checkout at `C:\mori-build` remains its read-only Git object cache.
Prepare an upgrade in a separate directory with:

```powershell
python windows/prepare_checkout.py --cache-checkout C:\mori-build
```

Keep the build checkout at a real short path: some tools resolve junctions while others do
not, which can break generated-file lookup. It contains `.gclient` and the
`src` directory. For dependency updates, pin the revision by commit:

```powershell
cd C:\mori-build-156
gclient sync --no-history --revision src@1d17a2e2ee147bb7d6ae2ab6abdb3a2510c17775
```

Sync dependencies **before** applying Mori; a later sync can overwrite patched
files. Use a fresh synced checkout if patches or their application state need
to be reset. From the Mori repository root:

```powershell
python windows/build.py --setup-only   # apply patches and generate build files
python windows/build.py --jobs 12      # configure and compile chrome.exe
```

For a native per-user Windows installer:

```powershell
python -m pip install Pillow
python windows/package.py --jobs 12
```

This builds Chromium's `mini_installer` target and writes
`dist/Mori-Setup-156.0.8078.47-x64.exe`, its SHA-256 checksum, and build metadata.
The application, installer, Windows shortcuts, and browser registration use
the Mori name. The Windows icon and product images come from the original
`mac/assets/AppIcon.png`; `windows/branding.py` converts the existing artwork
to multi-resolution Windows resources. The native installer includes the
runtime DLLs, provides uninstall support, and preserves browser profile data
across application updates. Local installer builds are unsigned.

To create the visible setup wizard, with Inno Setup 6 installed, run:

```powershell
python windows/build-wizard.py
```

This writes `dist/Mori-Setup-156.0.8078.47-x64-Wizard.exe` and its checksum.
Both installer builds verify their embedded version, Mori product name, and
original Mori icon before publishing the output.

`--checkout C:\another-short-path` selects a different checkout. The script
discovers Visual Studio with `vswhere`, uses Chromium's bundled compilers and
Ninja, and builds locally with component DLLs and no debug symbols. It applies
the shared ungoogled-chromium patches, domain substitution, Mori's patches,
and Windows compatibility fixes. Unlike tarball packaging scripts, it retains
the tools and dependencies already installed by `gclient`. Its patch state
is stored beside `src` in `mori-setup-state.json`; subsequent runs resume setup.
Build and package commands share a checkout lock, so parallel tasks queue their
builds rather than writing the same outputs concurrently.
The Mori source folder is linked into Chromium, so source edits are picked up
by incremental builds.

The output is `C:\mori-build-156\src\out\Default\chrome.exe`; keep the accompanying
DLLs and runtime files together. Run with an isolated test profile:

```powershell
C:\mori-build-156\src\out\Default\chrome.exe --user-data-dir="$env:TEMP\MoriTestProfile" --no-first-run
```

The Windows build enables `proprietary_codecs=true` and
`ffmpeg_branding="Chrome"` for H.264/AAC playback in MP4 and MediaSource
players, including X videos. These codec flags do not change Mori's branding.
After changing them, rebuild the browser and installer; an existing installed
browser keeps its old codec support until it is updated and restarted.
Verify codec detection and actual MP4/MediaSource decoding with:

```powershell
npm --prefix windows run test:media
```

The suite uses Chromium's local H.264/AAC fixture and isolated profiles.
`MORI_TEST_BROWSER` selects another browser executable, including an installed
copy of Mori. Install the test dependencies as described below.


### Windows validation

Install the pinned Playwright dependency and use its CLI through the package
scripts with Node.js 20 or newer. No separate Playwright browser download is needed: these tests use the
Mori executable from the checkout pinned in `windows/chromium.json`.

```powershell
python -m unittest discover -s windows/tests -p "test_*.py" -v
npm ci --prefix windows
npm --prefix windows test
```

`MORI_TEST_BROWSER` overrides the executable path. Use a copied runtime, with
all its DLLs and data files, when other tasks may rebuild the default output.
Both suites use isolated profiles and enable Chromium's native sandbox.
The media suite uses a progressive H.264/AAC MP4 for URL playback and a
fragmented fixture for MediaSource playback.

### Picture in Picture


Settings → Tabs → Picture in Picture controls the video hover button,
automatic entry when switching tabs, and returning automatic players to their
source tab. Automatic entry is off by default. Ctrl+Shift+] and the command
bar's Picture in Picture action also toggle the active video's player.

The video controls use Chromium's native PiP API and work with HTML video,
MediaSource players such as YouTube, and permitted embedded players. They
respect fullscreen mode, a video's `disablePictureInPicture` flag, and frame
permissions. Captions rendered separately by a website are not copied into the
native video window.

Run the headless integration suite against the built Windows executable with
the pinned Playwright CLI:

```powershell
npm --prefix windows run test:pip
```

`MORI_TEST_BROWSER` overrides the executable path, and `MORI_TEST_MEDIA`
overrides the Chromium `content/test/data/media/bear-320x240.webm` fixture.
Tests use isolated temporary profiles and do not use the installed browser's
profile.


## Linux: Chromium Views

Use an upstream `ungoogled-chromium-portablelinux` checkout, including its
submodules, at `ungoogled-chromium-linux/`. Its core and Chromium source must
match the version in `windows/chromium.json` before applying Mori's shared
patches. The scripts reject mismatched versions.

```sh
linux/fetch.sh
JOBS=12 linux/build.sh
linux/run.sh
```

The build uses Python, Node/npm, Ninja, gperf, Go, clang-format, and Chromium's
LLVM, Rust, and Linux sysroot. Follow the matching upstream packaging checkout's
host dependency requirements. `linux/build.sh --setup-only` applies Mori's
patches and generates Ninja files; `--toolchain-only` prepares the toolchain.

The Linux Views output is
`ungoogled-chromium-linux/build/src/out/Default/chrome`. See
[linux/README.md](linux/README.md) for the headless harness and native import and
passkey test targets. The current Chromium 156 patch port has been built on
Windows; Linux host verification remains pending.

## macOS: Chromium Views

The macOS build uses the same `linux/mori/` source, including the small native
adapter in `mori_window_events_mac.mm`. It builds `Chromium.app`, brands the
bundle as Mori, and packages `dist/Mori.app`.

Prepare an upstream macOS ungoogled-chromium packaging checkout, including its
submodules, at `ungoogled-chromium-mac/`. Use the matching Chromium version from
`windows/chromium.json` and the packaging checkout's Xcode/SDK requirements.
The source version guard runs before Mori patches are applied. These scripts
target Apple Silicon and require Python, Node/npm, Ninja, and esbuild; a local
`.venv-build` environment can provide Python build dependencies.

```sh
mac/fetch.sh
JOBS=10 mac/build.sh
mac/package.sh
open -n dist/Mori.app --args --user-data-dir="$TMPDIR/MoriViewsTest" --no-first-run
```

`mac/build.sh --setup-only` generates the build files without compiling.
`mac/package.sh /Applications/Mori.app` selects a different package destination.
Existing bundles go to the trash. Packaging applies the name and original Mori
icon, signs nested helpers and the framework with their existing entitlements,
and verifies the resulting bundle. Local ad-hoc signing is not notarization.
The current Chromium 156 port still needs macOS host verification.

The source artwork is `mac/assets/AppIcon.svg`; `mac/build-icon.sh` regenerates
PNG, ICNS, and the asset catalog with librsvg and Xcode. Windows packaging uses
`mac/assets/AppIcon.png`. Keep these assets when changing platform wrappers.

## iOS app

The independent iOS app is in `ios/`. Open
`ios/MoriBrowser.xcodeproj` on a Mac with Xcode; the `MoriBrowser` scheme includes
the application and tests. The checked-in project targets iOS 18 or newer and
uses WKWebView. Set an appropriate development team for device signing.
`ios/project.yml` is the XcodeGen project source.

This app retains SwiftUI; it does not depend on the removed desktop overlay.

## Logs and profiles

For the Views builds, pass `--enable-logging=stderr --v=0` to the executable.
Use `--user-data-dir` to keep experiments separate from normal browsing data.
On macOS the Views executable is `Mori.app/Contents/MacOS/Chromium`.
Keep all runtime DLLs/frameworks, resources, locales, and snapshots with their
executable. A new link does not update a running browser process: use a fresh
isolated process when checking a new build.
