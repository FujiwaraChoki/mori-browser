#!/bin/bash
# Rebuild the macOS icon family from the editable SVG master (librsvg required).
set -euo pipefail
_mac="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_work="$(mktemp -d "${TMPDIR:-/tmp}/mori-icon.XXXXXX")"
_iconset="${_work}/AppIcon.iconset"
mkdir -p "${_iconset}"
rsvg-convert -w 1024 -h 1024 "${_mac}/assets/AppIcon.svg" -o "${_mac}/assets/AppIcon.png"
for _size in 16 32 128 256 512; do
    /usr/bin/sips -z "${_size}" "${_size}" "${_mac}/assets/AppIcon.png" \
        --out "${_iconset}/icon_${_size}x${_size}.png" >/dev/null
    _retina=$((_size * 2))
    /usr/bin/sips -z "${_retina}" "${_retina}" "${_mac}/assets/AppIcon.png" \
        --out "${_iconset}/icon_${_size}x${_size}@2x.png" >/dev/null
done
/usr/bin/iconutil -c icns "${_iconset}" -o "${_mac}/assets/AppIcon.icns"
_catalog="${_work}/Assets.xcassets"
mkdir -p "${_catalog}"
mv "${_iconset}" "${_catalog}/AppIcon.appiconset"
{
    echo '{"images":['
    _separator=''
    for _size in 16 32 128 256 512; do
        for _scale in 1 2; do
            _suffix=''
            [ "${_scale}" = 1 ] || _suffix='@2x'
            printf '%s{"idiom":"mac","size":"%sx%s","scale":"%sx","filename":"icon_%sx%s%s.png"}' \
                "${_separator}" "${_size}" "${_size}" "${_scale}" "${_size}" "${_size}" "${_suffix}"
            _separator=','
        done
    done
    echo '],"info":{"version":1,"author":"xcode"}}'
} > "${_catalog}/AppIcon.appiconset/Contents.json"
xcrun actool "${_catalog}" --compile "${_work}" --platform macosx \
    --minimum-deployment-target 13.0 --app-icon AppIcon \
    --output-partial-info-plist "${_work}/icon-info.plist" >/dev/null
cp "${_work}/Assets.car" "${_mac}/assets/Assets.car"
trash "${_work}"
