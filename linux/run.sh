#!/bin/bash
# Runs the development build on the desktop (Wayland), with Mori's own
# profile in ~/.config/mori, as the "mori" app (its Wayland app ID). Extra
# arguments go to the browser.
H=$(cd "$(dirname "$0")/.." && pwd)
exec "$H/ungoogled-chromium-linux/build/src/out/Default/chrome" \
  --ozone-platform=wayland --no-default-browser-check --class=mori "$@"
