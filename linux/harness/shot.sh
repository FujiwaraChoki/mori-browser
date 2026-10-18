#!/bin/bash
H=$(cd "$(dirname "$0")" && pwd); R=${MORI_HARNESS_RUN:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/harness-run}; source $R/${1:-main}/env.sh; grim -o HEADLESS-1 "${2:-$HL/shot.png}"
