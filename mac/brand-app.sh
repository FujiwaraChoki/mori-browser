#!/bin/bash
# Apply Mori's Dock/Finder identity after Chromium assembles the app bundle.
set -euo pipefail
_mac="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_app="${1:?Usage: mac/brand-app.sh /path/to/Mori.app}"
_plist="${_app}/Contents/Info.plist"

test -f "${_plist}"
/usr/libexec/PlistBuddy -c 'Set :CFBundleDisplayName Mori' "${_plist}"
/usr/libexec/PlistBuddy -c 'Set :CFBundleName Mori' "${_plist}"
/usr/libexec/PlistBuddy -c 'Set :CFBundleIconFile app.icns' "${_plist}"
/usr/bin/plutil -replace CFBundleIconName -string AppIcon "${_plist}"
cp "${_mac}/assets/AppIcon.icns" "${_app}/Contents/Resources/app.icns"
cp "${_mac}/assets/Assets.car" "${_app}/Contents/Resources/Assets.car"
for _strings in "${_app}"/Contents/Resources/*.lproj/InfoPlist.strings; do
    [ -f "${_strings}" ] || continue
    /usr/bin/plutil -convert binary1 "${_strings}"
    /usr/bin/plutil -replace CFBundleDisplayName -string Mori "${_strings}"
    /usr/bin/plutil -replace CFBundleName -string Mori "${_strings}"
done

# Resource/plist changes invalidate a previously signed bundle.
/usr/bin/codesign --force --sign - --preserve-metadata=entitlements,flags,runtime "${_app}"
/usr/bin/touch "${_app}"
