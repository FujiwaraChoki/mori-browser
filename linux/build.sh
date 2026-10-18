#!/bin/bash
# Mori: set up Chromium's toolchain and build ungoogled-chromium for
# development (component build, no symbols) on this Arch machine, without
# Docker. Run after fetch.sh.
#
#   JOBS=12 ./build.sh          # build `chrome` with 12 parallel jobs
#   ./build.sh --setup-only     # toolchain + gn gen, no compile
#   ./build.sh --toolchain-only # download clang, Rust and the sysroot
set -euo pipefail
_mori_linux="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "${_mori_linux}/../ungoogled-chromium-linux/scripts/shared.sh"
setup_paths

JOBS=${JOBS:-12}
_tools="${_root}/build/host-tools"

# The portable script symlinks distro packages; on this machine some live in
# mise (node) or are installed into ${_tools} instead.
setup_toolchain_mori() {
    local stamp="${_src_dir}/.toolchain.stamp"
    if [ ! -f "${stamp}" ]; then
        "${_src_dir}/tools/rust/update_rust.py"
        "${_src_dir}/tools/clang/scripts/update.py"
        "${_src_dir}/build/linux/sysroot_scripts/install-sysroot.py" --arch=x64
        touch "${stamp}"
    fi
    if [ "${1:-}" = "--toolchain-only" ]; then
        echo "TOOLCHAIN DONE"
        exit 0
    fi
    for tool in node gperf go clang-format ninja npm; do
        command -v "${tool}" >/dev/null || { echo "missing build tool: ${tool}" >&2; exit 1; }
    done

    mkdir -p "${_src_dir}/third_party/node/linux/node-linux-x64/bin"
    ln -sf "$(command -v node)" "${_src_dir}/third_party/node/linux/node-linux-x64/bin/node"
    mkdir -p "${_src_dir}/third_party/gperf/cipd/bin/"
    ln -sf "$(command -v gperf)" "${_src_dir}/third_party/gperf/cipd/bin/gperf"
    mkdir -p "${_src_dir}/third_party/dawn/tools/golang/linux-amd64/bin"
    ln -sf "$(command -v go)" "${_src_dir}/third_party/dawn/tools/golang/linux-amd64/bin/go"
    mkdir -p "${_src_dir}/buildtools/linux64-format"
    ln -sf "$(command -v clang-format)" "${_src_dir}/buildtools/linux64-format/clang-format"

    # DevTools needs the esbuild version its package.json pins; the prebuilt
    # binary is pruned from the tarball, so install that version locally.
    local devtools="${_src_dir}/third_party/devtools-frontend/src"
    local esbuild_version
    esbuild_version=$(python3 -c "import json;print(json.load(open('${devtools}/node_modules/esbuild/package.json'))['version'])" 2>/dev/null || true)
    if [ -n "${esbuild_version}" ] && [ ! -d "${_tools}/esbuild-${esbuild_version}/node_modules/esbuild" ]; then
        mkdir -p "${_tools}/esbuild-${esbuild_version}"
        npm install --prefix "${_tools}/esbuild-${esbuild_version}" --no-save --no-audit --no-fund "esbuild@${esbuild_version}"
    fi
    if [ -n "${esbuild_version}" ]; then
        rm -rf "${devtools}/node_modules/esbuild"
        ln -s "${_tools}/esbuild-${esbuild_version}/node_modules/esbuild" "${devtools}/node_modules/esbuild"
        # The portable patch (chromium-155-use-system-esbuild) expects
        # Debian's /usr/bin/esbuild; use the matching local binary instead.
        sed -i "s|_esbuild = \"/usr/bin/esbuild\"|_esbuild = \"${_tools}/esbuild-${esbuild_version}/node_modules/@esbuild/linux-x64/bin/esbuild\"|" \
            "${devtools}/scripts/build/ninja/bundle.gni"
    fi

    # Likewise chromium-154-revert-typescript-changes expects /usr/bin/tsc
    # (the portable Docker image installs the latest typescript from npm).
    local ts_version=7.0.2
    if [ ! -x "${_tools}/typescript-${ts_version}/node_modules/.bin/tsc" ]; then
        npm install --prefix "${_tools}/typescript-${ts_version}" --no-save --no-audit --no-fund "typescript@${ts_version}"
    fi
    sed -i "s|tsc_binary = \"/usr/bin/tsc\"|tsc_binary = \"${_tools}/typescript-${ts_version}/node_modules/.bin/tsc\"|" \
        "${devtools}/scripts/build/typescript/typescript_vars.gni"

    local clang_bin="${_src_dir}/third_party/llvm-build/Release+Asserts/bin"
    export CC="${clang_bin}/clang" CXX="${clang_bin}/clang++" AR="${clang_bin}/llvm-ar" NM="${clang_bin}/llvm-nm"
    export LLVM_BIN="${clang_bin}"
    local resource_dir
    resource_dir="$("${CC}" --print-resource-dir)"
    export CXXFLAGS="${CXXFLAGS:-} -resource-dir=${resource_dir} -B${LLVM_BIN}"
    export CPPFLAGS="${CPPFLAGS:-} -resource-dir=${resource_dir} -B${LLVM_BIN}"
    export CFLAGS="${CFLAGS:-} -resource-dir=${resource_dir} -B${LLVM_BIN}"
}

# ungoogled-chromium's flags plus the portable Linux ones, switched from an
# official (LTO/PGO, very RAM-hungry) build to a development build.
write_dev_gn_args() {
    mkdir -p "${_out_dir}"
    {
        cat "${_main_repo}/flags.gn" "${_root}/flags.linux.gn" \
            | grep -v -E '^(is_official_build|symbol_level|blink_symbol_level)='
        cat <<'GN'
# --- Mori development overrides ---
is_official_build=false
is_debug=false
is_component_build=true
symbol_level=0
blink_symbol_level=0
v8_symbol_level=0
dcheck_always_on=false
# mold is the default for development builds but is not in the tarball;
# lld ships with Chromium's clang.
use_mold=false
target_cpu="x64"
v8_target_cpu="x64"
GN
    } > "${_out_dir}/args.gn"
    cat "${_out_dir}/args.gn"
}

# Mori's sources are linked into the tree; patches/*.patch hook them into
# Chromium and fix ungoogled-chromium build issues.
apply_mori_patches() {
    ln -sfn "${_mori_linux}/mori" "${_src_dir}/chrome/browser/ui/views/mori"
    local p
    for p in "${_mori_linux}"/patches/*.patch; do
        [ -e "${p}" ] || continue
        if patch -d "${_src_dir}" -p1 -R --dry-run -s -f < "${p}" >/dev/null 2>&1; then
            continue  # already applied
        fi
        patch -d "${_src_dir}" -p1 -N -s < "${p}"
    done
}

setup_toolchain_mori "${1:-}"
apply_mori_patches
write_dev_gn_args
cd "${_src_dir}"
if [ ! -x out/Default/gn ]; then
    ./tools/gn/bootstrap/bootstrap.py -o out/Default/gn --skip-generate-buildfiles
fi
./out/Default/gn gen out/Default --fail-on-unused-args

if [ "${1:-}" = "--setup-only" ]; then
    echo "SETUP DONE"
    exit 0
fi

# -l keeps ninja from starting jobs while the machine is already busy.
ninja -C out/Default -j "${JOBS}" -l "${JOBS}" chrome
echo "BUILD DONE"
