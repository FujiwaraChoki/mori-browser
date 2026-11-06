#!/bin/bash
# Installs the Linux build (out/Default from linux/build.sh) for the current
# user: the runtime files to $PREFIX/lib/mori, a `mori` launcher in
# $PREFIX/bin, and a desktop entry and icon, so Mori appears in the app
# launcher. It does not make Mori the default browser.
#
#   linux/install.sh                # PREFIX defaults to ~/.local
#   PREFIX=/opt/mori-test linux/install.sh
#
# The profile is ~/.config/mori, as with linux/run.sh. Run it again after a
# build to update; quit Mori first.
set -euo pipefail
H=$(cd "$(dirname "$0")/.." && pwd)
OUT="$H/ungoogled-chromium-linux/build/src/out/Default"
PREFIX=${PREFIX:-$HOME/.local}
DEST="$PREFIX/lib/mori"
MARK="$DEST/.mori-install"

[ -x "$OUT/chrome" ] || { echo "no build at $OUT/chrome: run linux/build.sh" >&2; exit 1; }
if [ -e "$DEST" ] && [ ! -e "$MARK" ]; then
    echo "$DEST exists and was not made by this script; not touching it" >&2
    exit 1
fi
if pgrep -f -- "^$DEST/chrome( |$)" >/dev/null; then
    echo "Mori is running from $DEST; quit it first" >&2
    exit 1
fi

# The files Chromium itself says the browser needs at run time.
deps=$(mktemp)
trap 'rm -f "$deps"' EXIT
if ! (cd "$OUT/../.." && out/Default/gn desc out/Default //chrome:chrome runtime_deps) > "$deps"; then
    echo "gn could not list the runtime files:" >&2
    cat "$deps" >&2
    exit 1
fi
sed -i 's#^\./##' "$deps"
sort -u -o "$deps" "$deps"
if grep -q '^\.\./' "$deps"; then
    echo "unexpected runtime file outside $OUT" >&2
    exit 1
fi
mkdir -p "$DEST"
touch "$MARK"
rsync -a --files-from="$deps" "$OUT/" "$DEST/"

mkdir -p "$PREFIX/bin" "$PREFIX/share/applications" \
    "$PREFIX/share/icons/hicolor/512x512/apps"
cat > "$PREFIX/bin/mori" <<EOF
#!/bin/sh
# Mori, installed by linux/install.sh. Profile: ~/.config/mori.
exec "$DEST/chrome" --ozone-platform=wayland --no-default-browser-check --class=mori "\$@"
EOF
chmod +x "$PREFIX/bin/mori"

icon="$PREFIX/share/icons/hicolor/512x512/apps/mori.png"
if command -v magick >/dev/null; then
    magick "$H/mac/assets/AppIcon.png" -resize 512x512 "$icon"
else
    cp "$H/mac/assets/AppIcon.png" "$icon"
fi

cat > "$PREFIX/share/applications/mori.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Mori
GenericName=Web Browser
Comment=Browse the web
Exec=$PREFIX/bin/mori %U
Icon=mori
Terminal=false
Categories=Network;WebBrowser;
MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;
StartupWMClass=mori
Actions=new-window;new-private-window;

[Desktop Action new-window]
Name=New Window
Exec=$PREFIX/bin/mori --new-window

[Desktop Action new-private-window]
Name=New Private Window
Exec=$PREFIX/bin/mori --incognito
EOF
update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" 2>/dev/null || true

echo "Installed $("$DEST/chrome" --version 2>/dev/null || echo Mori) to $DEST"
