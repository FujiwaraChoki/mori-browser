#!/bin/bash
# Usage: run.sh NAME [chrome args/URLs] — start the Chromium build in sway
# instance NAME with a throwaway profile ($HL/profile) and the test extension.
H=$(cd "$(dirname "$0")" && pwd); R=${MORI_HARNESS_RUN:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/harness-run}
source $R/$1/env.sh; shift
OUT=${MORI_OUT:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/build/src/out/Default}
{
  echo '#!/bin/bash'
  printf 'exec %q --ozone-platform=wayland --user-data-dir=%q --no-first-run --no-default-browser-check --mute-audio --load-extension=%q' \
    "$OUT/chrome" "$HL/profile" "$H/ext-test"
  for a in "$@"; do printf ' %q' "$a"; done
  printf ' > %q 2>&1\n' "$HL/chrome.log"
} > $HL/launch.sh
chmod +x $HL/launch.sh
swaymsg exec $HL/launch.sh >/dev/null
