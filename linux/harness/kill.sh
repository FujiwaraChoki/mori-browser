#!/bin/bash
# Usage: kill.sh NAME — stop the test browser of instance NAME (matched by
# its profile path, so other browsers are never touched).
H=$(cd "$(dirname "$0")" && pwd); R=${MORI_HARNESS_RUN:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/harness-run}
pkill -f -- "--user-data-dir=$R/$1/profile( |$)" && sleep 2; true
