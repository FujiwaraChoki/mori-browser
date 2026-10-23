#!/bin/bash
# Package the Views-based macOS build with its user-facing Mori identity.
set -euo pipefail
_mac="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_source="${_mac}/../ungoogled-chromium-mac/build/src/out/Default/Chromium.app"
_destination="${1:-${_mac}/../dist/Mori.app}"
if [[ "${_destination##*/}" != Mori.app ]]; then
    echo 'Destination must end in Mori.app so Finder and the Dock use Mori.' >&2
    exit 1
fi
test -d "${_source}"
mkdir -p "$(dirname "${_destination}")"
if [ -e "${_destination}" ]; then
    trash "${_destination}"
fi
/usr/bin/ditto "${_source}" "${_destination}"
# Incremental Chromium links leave linker-only signatures on nested bundles.
# Sign inside out and preserve each helper's existing entitlements.
for _helper in "${_destination}"/Contents/Frameworks/"Chromium Framework.framework"/Versions/Current/Helpers/*.app; do
    [ -d "${_helper}" ] || continue
    /usr/bin/codesign --force --sign - --preserve-metadata=entitlements,flags,runtime "${_helper}"
done
/usr/bin/codesign --force --sign - --preserve-metadata=entitlements,flags,runtime \
    "${_destination}/Contents/Frameworks/Chromium Framework.framework"
"${_mac}/brand-app.sh" "${_destination}"
/usr/bin/codesign --verify --deep --strict "${_destination}"
echo "Packaged ${_destination}"
