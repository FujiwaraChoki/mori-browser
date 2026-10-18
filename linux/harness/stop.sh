#!/bin/bash
H=$(cd "$(dirname "$0")" && pwd); R=${MORI_HARNESS_RUN:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/harness-run}; kill $(cat $R/${1:-main}/sway.pid) 2>/dev/null; echo stopped ${1:-main}
