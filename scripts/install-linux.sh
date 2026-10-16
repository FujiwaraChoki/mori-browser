#!/usr/bin/env bash
# Install a bundle from scripts/package-linux.sh for the current user:
#   ~/.local/opt/mori, ~/.local/bin/mori, desktop entry and icon.
# Usage: scripts/install-linux.sh [bundle-dir]
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
bundle="${1:-$root/dist/mori-linux-$(uname -m)}"
[ -x "$bundle/mori" ] || { echo "No bundle at $bundle; run scripts/package-linux.sh first" >&2; exit 1; }

prefix="$HOME/.local"
dest="$prefix/opt/mori"
if [ -e "$dest" ]; then
  gio trash "$dest"
fi
mkdir -p "$prefix/opt" "$prefix/bin" "$prefix/share/applications" "$prefix/share/icons/hicolor/scalable/apps"
cp -rp "$bundle" "$dest"
ln -sf "$dest/mori" "$prefix/bin/mori"
sed "s|^Exec=mori|Exec=$dest/mori|" "$bundle/share/applications/mori.desktop" \
  > "$prefix/share/applications/mori.desktop"
cp "$bundle/share/icons/hicolor/scalable/apps/mori.svg" "$prefix/share/icons/hicolor/scalable/apps/mori.svg"
command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true

echo "Installed Mori to $dest (launcher: $prefix/bin/mori)"
