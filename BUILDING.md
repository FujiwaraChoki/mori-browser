# Building Mori Browser

This repository holds two implementations of Mori:

- **Mori 3 (Rust, cross-platform)** - a GPUI chrome around Chromium embedded
  through CEF, in the Cargo workspace under `crates/`. This is the build that
  runs on Linux.
- **Mori 2 (macOS)** - the SwiftUI/AppKit chrome compiled into an
  ungoogled-chromium checkout. See [Mori 2 on macOS](#mori-2-on-macos) below.

## Mori 3 on Linux

### Prerequisites

- Rust (stable, edition 2024; `rustup` toolchain 1.98 or newer).
- A Vulkan-capable GPU driver (GPUI renders with wgpu/Vulkan).
- `cmake`, a C++ compiler, and the usual X11/Wayland/xkbcommon development
  headers (on Arch: `base-devel cmake libxkbcommon wayland libx11`).
- About 2 GB of disk for the CEF binary distribution.

### Build and run

```sh
cargo run --release
```

The first build downloads the matching CEF distribution (Chromium 154) into
`.cef/` at the repository root (configured in `.cargo/config.toml`). The CEF
runtime files are copied next to the binary in `target/<profile>/`, so the
binary runs straight from there:

```sh
target/release/mori                 # restores the last session
target/release/mori example.com     # also opens a tab
```

Starting `mori` again while it runs hands the URLs to the running instance
(Chromium's process singleton) and exits.

### Package and install

```sh
scripts/package-linux.sh            # -> dist/mori-linux-x86_64/
scripts/install-linux.sh            # -> ~/.local/opt/mori, ~/.local/bin/mori,
                                    #    desktop entry and icon
```

Both scripts move an existing output to the trash with `gio trash`; they never
delete with `rm -rf`.

### Tests

```sh
cargo test -p mori-core -p mori-engine -p mori
cargo clippy --workspace
```

`mori-core` (workspace, stores, omnibox, launcher ranking, theme math, page
scripts) is UI-free and carries most of the unit tests.

### Runtime environment

| Variable | Effect |
|---|---|
| `MORI_DATA_DIR` | Use another data directory (default `~/.local/share/mori`). Handy for an isolated test profile. |
| `MORI_START_URL` | Start with a single tab at this URL instead of restoring the session. |
| `MORI_NO_SANDBOX` | Disable Chromium's sandbox (only for systems without user namespaces). |
| `MORI_OZONE_PLATFORM` | Chromium's ozone backend (default `headless`: pages are drawn by Mori and Mori bridges the clipboard itself). |
| `MORI_REMOTE_DEBUGGING_PORT` | Expose the Chrome DevTools Protocol on this port. |
| `MORI_CODEX_DYNAMIC_TOOLS=0`, `MORI_ENABLE_CODEX_ASSISTANT=0` | Assistant switches (see the assistant panel). |
| `RUST_LOG` | Log filter, e.g. `warn,mori=info,mori_engine=debug`. |

Data lives in `$MORI_DATA_DIR`: `session.json` (spaces, folders, tabs),
`settings.json`, `history.json`, `bookmarks.json`, `archive.json`,
`routes.json`, `boosts.json`, `favicons/`, and Chromium's profile in
`profile/`.

## Mori 2 on macOS

The macOS build is the Mori browser work on top of ungoogled-chromium.

### Layout

- `ungoogled-chromium-macos/build/src` is the Chromium source tree used for
  day-to-day builds.
- `out/Default/Mori.app` inside that source tree is the local build output.
- `$HOME/Downloads/MoriBrowser.app` is the packaged app used for manual testing.
- `depot_tools` and `binshims` are checked out beside the source tree and should
  be placed on `PATH` for builds.

### Prerequisites

- macOS with Xcode installed at `/Applications/Xcode.app`.
- The Chromium checkout and generated `out/Default` build directory already
  present under `ungoogled-chromium-macos/build/src`.
- Enough free disk space for Chromium link output and dSYMs.

### Build

Run from the repository root:

```sh
repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root/ungoogled-chromium-macos/build/src"
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
export PATH="$repo_root/binshims:$repo_root/depot_tools:$PATH"
ninja -j 16 -l 24 -C out/Default chrome
```

The normal successful build still prints warnings from Swift deployment-target
mismatch and `CoreAudioTypes` auto-linking. Treat ninja exit code `0` as the
source of truth.

### Package To Downloads

After a successful build:

```sh
repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root/ungoogled-chromium-macos/build/src"
if [ -e "$HOME/Downloads/MoriBrowser.app" ]; then
  trash "$HOME/Downloads/MoriBrowser.app"
fi
/usr/bin/ditto "out/Default/Mori.app" "$HOME/Downloads/MoriBrowser.app"
```

Use `trash`, not `rm -rf`, when replacing old app bundles.

Verify the packaged framework timestamp:

```sh
find "$HOME/Downloads/MoriBrowser.app/Contents/Frameworks" \
  -type f \( -name 'Chromium Framework' -o -name 'Mori Framework' \) \
  -exec stat -f '%Sm %N' -t '%Y-%m-%d %H:%M:%S' {} \;
```

### Run

Open the packaged app:

```sh
/usr/bin/open -n "$HOME/Downloads/MoriBrowser.app"
```

For isolated testing without reusing the main Chromium profile:

```sh
profile="$HOME/Library/Application Support/MoriBrowserTestProfile"
mkdir -p "$profile"
/usr/bin/open -n "$HOME/Downloads/MoriBrowser.app" --args \
  --user-data-dir="$profile" \
  --no-first-run
```

Check that it stayed running:

```sh
pgrep -fl "$HOME/Downloads/MoriBrowser.app/Contents/MacOS/Mori"
```

### Logging

For keyboard shortcut diagnostics, current instrumented builds emit `MORI-KEY`
messages. To capture launch logs directly:

```sh
"$HOME/Downloads/MoriBrowser.app/Contents/MacOS/Mori" \
  --enable-logging=stderr \
  --v=0
```

You can also confirm an instrumented packaged binary contains the diagnostics:

```sh
find "$HOME/Downloads/MoriBrowser.app/Contents/Frameworks" \
  -type f \( -name 'Chromium Framework' -o -name 'Mori Framework' \) \
  -exec strings -a {} \; | grep -m 5 'MORI-KEY'
```

### Avoiding Stale Binaries

If an already-running browser process is reused, it may still have the old
framework loaded. For shortcut testing, prefer a fresh `open -n` launch with a
separate `--user-data-dir`, or quit the existing app before testing the newly
packaged bundle.

### Common Build Loop

```sh
repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root/ungoogled-chromium-macos/build/src"
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
export PATH="$repo_root/binshims:$repo_root/depot_tools:$PATH"
ninja -j 16 -l 24 -C out/Default chrome

if [ -e "$HOME/Downloads/MoriBrowser.app" ]; then
  trash "$HOME/Downloads/MoriBrowser.app"
fi
/usr/bin/ditto "out/Default/Mori.app" "$HOME/Downloads/MoriBrowser.app"
/usr/bin/open -n "$HOME/Downloads/MoriBrowser.app"
```
