#!/bin/bash
# Mori: unpack, prune, patch and domain-substitute the Chromium sources for
# macOS (no build), and fetch the prebuilt LLVM, Rust and Node it builds
# with. Needs the upstream macOS packaging checkout with submodules at the
# shared Chromium version in ../ungoogled-chromium-mac.
#
# Uses Chromium's source tarball, as the Linux build does: put
# chromium-<version>-lite.tar.xz (and its .hashes) in
# ungoogled-chromium-mac/build/download_cache to skip the download.
set -euo pipefail
_mori="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
_root="${_mori}/ungoogled-chromium-mac"
_src="${_root}/build/src"
_main="${_root}/ungoogled-chromium"

# The build tools want Python 3.13 or older, with PySocks and httplib2
# (see BUILDING.md).
[ -x "${_mori}/.venv-build/bin/python3" ] && export PATH="${_mori}/.venv-build/bin:${PATH}"
export PATH="/opt/homebrew/bin:${PATH}"
python3 "${_mori}/linux/tools/check_chromium_version.py" --base "${_main}"

mkdir -p "${_root}/build/download_cache"
"${_root}/retrieve_and_unpack_resource.py" -d -g arm64
python3 "${_main}/utils/prune_binaries.py" "${_src}" "${_main}/pruning.list"
python3 "${_main}/utils/patches.py" apply "${_src}" "${_main}/patches" "${_root}/patches"
python3 "${_main}/utils/domain_substitution.py" apply \
    -r "${_main}/domain_regex.list" -f "${_main}/domain_substitution.list" "${_src}"
"${_root}/retrieve_and_unpack_resource.py" -p arm64
echo "FETCH DONE"
