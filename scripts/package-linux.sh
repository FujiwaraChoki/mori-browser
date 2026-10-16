#!/usr/bin/env bash
# Build a self-contained Mori bundle for Linux:
#   dist/mori-linux-<arch>/  (mori binary + CEF runtime + resources)
# Usage: scripts/package-linux.sh [output-dir]
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

cargo build --release -p mori

arch="$(uname -m)"
out="${1:-$root/dist/mori-linux-$arch}"
release="$root/target/release"

if [ -e "$out" ]; then
  # Never rm -rf: move the previous bundle to the trash.
  gio trash "$out"
fi
mkdir -p "$out"

install -m 755 "$release/mori" "$out/mori"
for f in libcef.so libvk_swiftshader.so libvulkan.so.1 vk_swiftshader_icd.json \
         icudtl.dat v8_context_snapshot.bin resources.pak \
         chrome_100_percent.pak chrome_200_percent.pak chrome-sandbox; do
  cp -p "$release/$f" "$out/$f"
done
cp -rp "$release/locales" "$out/locales"

# CEF ships libcef.so with debug info (~1.4 GB); strip it for distribution.
if command -v strip >/dev/null; then
  strip --strip-debug "$out/libcef.so" "$out/libvk_swiftshader.so"
  strip "$out/mori"
fi

mkdir -p "$out/share/applications" "$out/share/icons/hicolor/scalable/apps"
cp "$root/packaging/linux/mori.desktop" "$out/share/applications/mori.desktop"
cp "$root/crates/mori/assets/mori/logo.svg" "$out/share/icons/hicolor/scalable/apps/mori.svg"

echo "Bundled Mori into $out ($(du -sh "$out" | cut -f1))"
