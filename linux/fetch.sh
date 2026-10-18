#!/bin/bash
# Mori: download, unpack, prune, patch and domain-substitute the Chromium sources (no build).
# Needs a clone of ungoogled-chromium-portablelinux in ../ungoogled-chromium-linux.
set -euo pipefail
. "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/ungoogled-chromium-linux/scripts/shared.sh"
setup_paths
fetch_sources false false
apply_patches
apply_domsub
fix_tool_downloading
echo "FETCH DONE"
