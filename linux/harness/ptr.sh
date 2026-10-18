#!/bin/bash
# Usage: ptr.sh NAME move|click|dblclick|scroll|drag X Y [right|DY|X2 Y2] — virtual pointer in
# sway instance NAME. X/Y are screenshot (physical) pixels.
H=$(cd "$(dirname "$0")" && pwd); R=${MORI_HARNESS_RUN:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/harness-run}
source $R/$1/env.sh; shift
[ -x "$H/vptr/target/release/vptr" ] || (cd "$H/vptr" && cargo build --release --offline -q)
size=$(swaymsg -t get_outputs -r | python3 -c "import json,sys; o=json.load(sys.stdin)[0]; print(f\"{o['current_mode']['width']}x{o['current_mode']['height']}\")")
VPTR_SIZE=$size exec "$H/vptr/target/release/vptr" "$@"
