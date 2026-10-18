#!/bin/bash
# Usage: key.sh NAME <wtype args>  (throwaway Shift first: wtype drops the first key after installing its keymap)
H=$(cd "$(dirname "$0")" && pwd); R=${MORI_HARNESS_RUN:-$(cd "$H/../.." && pwd)/ungoogled-chromium-linux/harness-run}; source $R/$1/env.sh; shift; wtype -k Shift_L "$@"
