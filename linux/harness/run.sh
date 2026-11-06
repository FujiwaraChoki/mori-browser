#!/bin/bash
# Usage: run.sh NAME [chrome args/URLs] — start the Chromium build in sway
# instance NAME with a throwaway profile ($HL/profile) and the test extension.
# FIXTURE=1 also loads fixture-ext, which pins and groups tabs on first run;
# NOEXT=1 loads no extension (for testing without the fixture UI).
#
# The test browser must never reach the real desktop. It only sees this sway's
# Wayland display, and gets no session bus, so a file chooser or notification
# cannot go through the user's xdg portal. Downloads save to $HL/downloads with
# no prompt. Handing a URL or file to another app is logged, not run: see
# $HL/desktop-blocked.log. It also gets its own home ($HL/home), so file dialogs
# and imports cannot see the user's files or their other browsers' data.
# Everything but loopback goes to a proxy on a closed local port (Chromium sends
# loopback direct), so a test cannot contact a real site, and the Codex CLI is
# stubbed, so the assistant cannot use a real account.
H=$(cd "$(dirname "$0")" && pwd); R=${MORI_HARNESS_RUN:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/harness-run}
source $R/$1/env.sh; shift
OUT=${MORI_OUT:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/build/src/out/Default}

if pgrep -f -- "--user-data-dir=$HL/profile( |$)" >/dev/null; then
  echo "run.sh: a browser is already using $HL/profile" >&2
fi

# Chromium rewrites Preferences while it runs, so set these before launch.
mkdir -p "$HL/profile/Default" "$HL/downloads" "$HL/home"
python3 - "$HL/profile/Default/Preferences" "$HL/downloads" <<'PY'
import json, sys
path, downloads = sys.argv[1:]
try:
    with open(path) as f:
        prefs = json.load(f)
except (OSError, ValueError):
    prefs = {}
prefs.setdefault("download", {}).update(
    prompt_for_download=False, default_directory=downloads, directory_upgrade=True)
prefs.setdefault("savefile", {})["default_directory"] = downloads
with open(path, "w") as f:
    json.dump(prefs, f)
PY

# Chromium opens things in other apps through these helpers.
mkdir -p "$HL/guard-bin"
for tool in xdg-open xdg-email xdg-settings xdg-mime gio gtk-launch kdialog zenity codex; do
  cat > "$HL/guard-bin/$tool" <<EOF
#!/bin/sh
echo "\$(date +%T) $tool \$*" >> "$HL/desktop-blocked.log"
exit 1
EOF
  chmod +x "$HL/guard-bin/$tool"
done

{
  echo '#!/bin/bash'
  echo 'unset HYPRLAND_INSTANCE_SIGNATURE HYPRLAND_CMD DESKTOP_STARTUP_ID XDG_ACTIVATION_TOKEN'
  # sway exec sets DISPLAY to its own Xwayland. A Wayland browser has no use for it, and the GPU
  # stack connecting to it now and then trips the suite's X11 check.
  echo 'unset DISPLAY XAUTHORITY'
  # An unset address would fall back to the user's bus in XDG_RUNTIME_DIR.
  echo 'export DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent/mori-harness-no-bus'
  echo 'export GTK_USE_PORTAL=0 NO_AT_BRIDGE=1'
  printf 'export HOME=%q\n' "$HL/home"
  echo 'unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_CACHE_HOME XDG_STATE_HOME'
  printf 'export PATH=%q:"$PATH"\n' "$HL/guard-bin"
  printf 'exec %q --ozone-platform=wayland --user-data-dir=%q --no-first-run --no-default-browser-check --mute-audio --password-store=basic --proxy-server=http://127.0.0.1:9' \
    "$OUT/chrome" "$HL/profile"
  [ -n "${NOEXT:-}" ] || printf ' --load-extension=%q' "$H/ext-test${FIXTURE:+,$H/fixture-ext}"
  for a in "$@"; do printf ' %q' "$a"; done
  printf ' > %q 2>&1\n' "$HL/chrome.log"
} > $HL/launch.sh
chmod +x $HL/launch.sh
swaymsg exec $HL/launch.sh >/dev/null
