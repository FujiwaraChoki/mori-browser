#!/usr/bin/env bash
set -euo pipefail

# Package a freshly built Mori.app into a personal (ad-hoc signed) DMG.
# Not for distribution — no notarization. The app already carries logo.png as
# its icon via the asset catalog; here we also brand the DMG volume with it.
#
# Usage: CONFIGURATION=Release ./Scripts/make_dmg.sh

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CONFIGURATION="${CONFIGURATION:-Release}"
APP="$ROOT/build/Build/Products/$CONFIGURATION/Mori.app"
VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist" 2>/dev/null || echo 0.1.0)"
DMG="$ROOT/dist/Mori-$VERSION.dmg"
VOL="Mori $VERSION"

[ -d "$APP" ] || { echo "✗ App not found at $APP — build it first." >&2; exit 1; }

echo "▶ Staging $APP"
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/Mori.app"
ln -s /Applications "$STAGE/Applications"

# Brand the DMG volume with logo.png (icns built on the fly).
if [ -f "$ROOT/logo.png" ]; then
  echo "▶ Building volume icon from logo.png"
  ICONSET="$(mktemp -d)/icon.iconset"; mkdir -p "$ICONSET"
  for s in 16 32 128 256 512; do
    sips -z $s $s        "$ROOT/logo.png" --out "$ICONSET/icon_${s}x${s}.png"     >/dev/null
    sips -z $((s*2)) $((s*2)) "$ROOT/logo.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET" -o "$STAGE/.VolumeIcon.icns"
  SetFile -a C "$STAGE" 2>/dev/null || true
fi

echo "▶ Creating $DMG"
mkdir -p "$ROOT/dist"
rm -f "$DMG"
hdiutil create \
  -volname "$VOL" \
  -srcfolder "$STAGE" \
  -fs HFS+ \
  -format UDZO \
  "$DMG"

rm -rf "$STAGE"
echo "✓ DMG ready: $DMG"
