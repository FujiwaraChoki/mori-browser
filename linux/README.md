# Mori on Linux (Chromium build)

Mori's Linux browser built into ungoogled-chromium. See
`docs/DECISIONS-linux-chromium.md` for why and how.

- `mori/`: Mori's C++ (Views) sources. `build.sh` links this folder into the
  Chromium tree as `chrome/browser/ui/views/mori`.
- `patches/`: changes to Chromium files, applied by `build.sh`: the hooks that
  call into `mori/`, and build fixes for ungoogled-chromium.
- `fetch.sh`, `build.sh`: download and build. They expect a clone of
  [ungoogled-chromium-portablelinux](https://github.com/ungoogled-software/ungoogled-chromium-portablelinux)
  (with submodules) in `../ungoogled-chromium-linux` (git-ignored).
- `harness/`: headless sway for driving the browser in tests without touching
  the desktop. State goes to `../ungoogled-chromium-linux/harness-run`.

```sh
linux/fetch.sh                      # once per Chromium version
JOBS=12 linux/build.sh              # build out/Default/chrome
```
