#!/bin/bash
# Usage: run.sh NAME [chrome args/URLs] — start the Chromium build in sway
# instance NAME with a throwaway profile ($HL/profile) and the test extension.
# FIXTURE=1 also loads fixture-ext, which pins and groups tabs on first run;
# NOEXT=1 loads no extension (for testing without the fixture UI).
H=$(cd "$(dirname "$0")" && pwd); R=${MORI_HARNESS_RUN:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/harness-run}
source $R/$1/env.sh; shift
OUT=${MORI_OUT:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/build/src/out/Default}
{
  echo '#!/bin/bash'
  printf 'exec %q --ozone-platform=wayland --user-data-dir=%q --no-first-run --no-default-browser-check --mute-audio' \
    "$OUT/chrome" "$HL/profile"
  [ -n "${NOEXT:-}" ] || printf ' --load-extension=%q' "$H/ext-test${FIXTURE:+,$H/fixture-ext}"
  for a in "$@"; do printf ' %q' "$a"; done
  printf ' > %q 2>&1\n' "$HL/chrome.log"
} > $HL/launch.sh
chmod +x $HL/launch.sh
swaymsg exec $HL/launch.sh >/dev/null
