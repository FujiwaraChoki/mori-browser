#!/bin/bash
# Mori: download, unpack, prune, patch and domain-substitute the Chromium sources (no build).
# Needs a clone of ungoogled-chromium-portablelinux in ../ungoogled-chromium-linux.
set -euo pipefail
. "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/ungoogled-chromium-linux/scripts/shared.sh"
setup_paths
python3 "$(dirname "${BASH_SOURCE[0]}")/tools/check_chromium_version.py" \
    --base "${_main_repo}"
fetch_sources false false
apply_patches
apply_domsub
fix_tool_downloading
echo "FETCH DONE"
