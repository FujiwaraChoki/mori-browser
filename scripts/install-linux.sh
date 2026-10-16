#!/usr/bin/env bash
# Install a bundle from scripts/package-linux.sh for the current user:
#   ~/.local/opt/mori, ~/.local/bin/mori, desktop entry and icon.
# Usage: scripts/install-linux.sh [--default] [bundle-dir]
#   --default  also make Mori the default web browser (xdg-settings)
set -euo pipefail

make_default=0
if [ "${1:-}" = "--default" ]; then
  make_default=1
  shift
fi

root="$(git rev-parse --show-toplevel)"
bundle="${1:-$root/dist/mori-linux-$(uname -m)}"
[ -x "$bundle/mori" ] || { echo "No bundle at $bundle; run scripts/package-linux.sh first" >&2; exit 1; }

prefix="$HOME/.local"
data="${XDG_DATA_HOME:-$prefix/share}"
dest="$prefix/opt/mori"
if [ -e "$dest" ]; then
  gio trash "$dest"
fi
mkdir -p "$prefix/opt" "$prefix/bin" "$data/applications" "$data/icons/hicolor/scalable/apps"
cp -rp "$bundle" "$dest"
ln -sf "$dest/mori" "$prefix/bin/mori"

# Every Exec line (the main entry and the New Window / New Private Window
# actions) starts the installed binary; a path with spaces is quoted.
exec_path="$dest/mori"
case "$exec_path" in
  *[[:space:]]*) exec_path="\"$exec_path\"" ;;
esac
sed "s|^Exec=mori|Exec=$exec_path|" "$bundle/share/applications/mori.desktop" \
  > "$data/applications/mori.desktop"
cp "$bundle/share/icons/hicolor/scalable/apps/mori.svg" "$data/icons/hicolor/scalable/apps/mori.svg"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true

if [ "$make_default" = 1 ]; then
  if xdg-settings set default-web-browser mori.desktop; then
    echo "Mori is now the default web browser"
  else
    echo "xdg-settings could not make Mori the default browser; use Settings > General in Mori" >&2
  fi
fi

echo "Installed Mori to $dest (launcher: $prefix/bin/mori)"
