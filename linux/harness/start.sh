#!/bin/bash
# Usage: start.sh NAME — isolated headless sway (1440x900); writes $H/NAME/env.sh
H=$(cd "$(dirname "$0")" && pwd); R=${MORI_HARNESS_RUN:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/harness-run}; N=${1:-main}; D=$R/$N; mkdir -p $D
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c "$H/sway.conf" > "$D/sway.log" 2>&1 &
PID=$!; echo $PID > "$D/sway.pid"
for i in $(seq 1 80); do sock=$(ls $XDG_RUNTIME_DIR/sway-ipc.*.$PID.sock 2>/dev/null | head -1); [ -n "$sock" ] && break; sleep 0.1; done
rm -f "$D/disp"; SWAYSOCK=$sock swaymsg exec "echo \$WAYLAND_DISPLAY > $D/disp" >/dev/null
for i in $(seq 1 30); do [ -s "$D/disp" ] && break; sleep 0.1; done
printf 'export SWAYSOCK=%s\nexport WAYLAND_DISPLAY=%s\nexport HL=%s\n' "$sock" "$(cat $D/disp)" "$D" > $D/env.sh
cat $D/env.sh
