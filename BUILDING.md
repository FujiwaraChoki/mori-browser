# Building Mori Browser

This repository now contains the Mori browser work on top of ungoogled-chromium
instead of the old CEF app.

## Layout

- `ungoogled-chromium-macos/build/src` is the Chromium source tree used for
  day-to-day builds.
- `out/Default/Mori.app` inside that source tree is the local build output.
- `$HOME/Downloads/MoriBrowser.app` is the packaged app used for manual testing.
- `depot_tools` and `binshims` are checked out beside the source tree and should
  be placed on `PATH` for builds.

## Prerequisites

- macOS with Xcode installed at `/Applications/Xcode.app`.
- The Chromium checkout and generated `out/Default` build directory already
  present under `ungoogled-chromium-macos/build/src`.
- Enough free disk space for Chromium link output and dSYMs.

## Build

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

## Package To Downloads

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

## Run

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

## Logging

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

## Avoiding Stale Binaries

If an already-running browser process is reused, it may still have the old
framework loaded. For shortcut testing, prefer a fresh `open -n` launch with a
separate `--user-data-dir`, or quit the existing app before testing the newly
packaged bundle.

## Common Build Loop

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
