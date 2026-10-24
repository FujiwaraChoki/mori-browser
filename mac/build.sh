#!/bin/bash
# Mori: build ungoogled-chromium for macOS with Mori's Views code in it, for
# development (no symbols, no LTO or PGO). Run after fetch.sh.
#
# Not a component build: with the prebuilt Rust toolchain only one shared
# library gets Rust's allocator shim, and the others fail to link
# (undefined __rust_alloc).
#
#   JOBS=10 mac/build.sh        # build `chrome` (out/Default/Chromium.app)
#   mac/build.sh --setup-only   # Mori's patches and gn gen, no compile
set -euo pipefail
_mori="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
_root="${_mori}/ungoogled-chromium-mac"
_src="${_root}/build/src"
_main="${_root}/ungoogled-chromium"
_out="${_src}/out/Default"

[ -x "${_mori}/.venv-build/bin/python3" ] && export PATH="${_mori}/.venv-build/bin:${PATH}"
export PATH="/opt/homebrew/bin:${PATH}"
python3 "${_mori}/linux/tools/check_chromium_version.py" \
    --base "${_main}" --source "${_src}"
JOBS=${JOBS:-10}

# Mori's sources are linked into the tree; linux/patches/*.patch hook them
# into Chromium (the macOS paths are in the same patches, and 0025).
apply_mori_patches() {
    ln -sfn "${_mori}/linux/mori" "${_src}/chrome/browser/ui/views/mori"
    local p
    for p in "${_mori}"/linux/patches/*.patch; do
        [ -e "${p}" ] || continue
        if patch -d "${_src}" -p1 -R --dry-run -s -f < "${p}" >/dev/null 2>&1; then
            continue  # already applied
        fi
        patch -d "${_src}" -p1 -N -s < "${p}"
    done
}

# ungoogled-chromium's flags plus the macOS ones, switched from an official
# (LTO/PGO) build to a development build.
write_dev_gn_args() {
    mkdir -p "${_out}"
    {
        cat "${_main}/flags.gn" "${_root}/flags.macos.gn" \
            | grep -v -E '^(is_official_build|symbol_level|blink_symbol_level|chrome_pgo_phase|use_thin_lto)='
        cat <<'GN'
# --- Mori development overrides ---
is_official_build=false
is_debug=false
is_component_build=false
symbol_level=0
blink_symbol_level=0
v8_symbol_level=0
dcheck_always_on=false
use_thin_lto=false
chrome_pgo_phase=0
# The prebuilt clang reads Chromium's precompile.h as a C++20 module
# interface; official builds use no precompiled headers either.
enable_precompiled_headers=false
# The SDK is newer than the Chromium release was built against.
treat_warnings_as_errors=false
target_cpu="arm64"
GN
    } > "${_out}/args.gn.new"
    # Unchanged args keep gn from regenerating everything.
    if ! cmp -s "${_out}/args.gn.new" "${_out}/args.gn"; then
        mv "${_out}/args.gn.new" "${_out}/args.gn"
    else
        trash "${_out}/args.gn.new"
    fi
}

apply_mori_patches
write_dev_gn_args
cd "${_src}"

# DevTools bundles with the esbuild from Homebrew, as upstream's build.sh.
_esbuild_version="$(esbuild --version)"
_esbuild_module="${_root}/build/esbuild-node/node_modules/esbuild"
if [[ ! -f "${_esbuild_module}/package.json" ]] || \
    [[ "$(node -p 'require(process.argv[1]).version' "${_esbuild_module}/package.json")" != "${_esbuild_version}" ]]; then
    npm install --prefix "${_root}/build/esbuild-node" --no-save --package-lock=false \
        --no-bin-links --ignore-scripts --omit=optional --no-audit --no-fund \
        "esbuild@${_esbuild_version}"
fi
if [ -e third_party/devtools-frontend/src/node_modules/esbuild ] || [ -L third_party/devtools-frontend/src/node_modules/esbuild ]; then
    trash third_party/devtools-frontend/src/node_modules/esbuild
fi
ln -s "${_esbuild_module}" third_party/devtools-frontend/src/node_modules/esbuild

[ -x out/Default/gn ] || ./tools/gn/bootstrap/bootstrap.py -o out/Default/gn --skip-generate-buildfiles
[ -x third_party/rust-toolchain/bin/bindgen ] || ./tools/rust/build_bindgen.py --skip-test
./out/Default/gn gen out/Default --fail-on-unused-args

if [ "${1:-}" = "--setup-only" ]; then
    echo "SETUP DONE"
    exit 0
fi

# -l keeps ninja from starting jobs while the Mac is already busy.
nice -n 10 ninja -C out/Default -j "${JOBS}" -l "${JOBS}" chrome
"${_mori}/mac/brand-app.sh" "${_out}/Chromium.app"
echo "BUILD DONE"
