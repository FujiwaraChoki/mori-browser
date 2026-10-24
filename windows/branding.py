"""Apply the existing Mori artwork to Windows resources, without retouching it."""
import io
from pathlib import Path

from PIL import Image


def write_changed(path, data):
    if not path.exists() or path.read_bytes() != data:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def apply_branding(repo, src):
    with Image.open(repo / 'mac/assets/AppIcon.png') as artwork:
        artwork = artwork.convert('RGBA')
        icon = io.BytesIO()
        artwork.save(icon, format='ICO', sizes=[(s, s) for s in (16, 20, 24, 32, 40, 48, 64, 128, 256)])
        icon_bytes = icon.getvalue()
        write_changed(repo / 'windows/assets/mori.ico', icon_bytes)
        for relative in (
            'chrome/app/theme/chromium/win/chromium.ico',
            'chrome/app/theme/chromium/win/app_list.ico',
            'chrome/app/theme/chromium/win/chromium_doc.ico',
            'chrome/app/theme/chromium/win/chromium_pdf.ico',
            'chrome/app/theme/chromium/win/isolated.ico',
            'chrome/app/theme/chromium/win/omnibox_everywhere.ico',
            'chrome/installer/mini_installer/mini_installer.ico',
            'chrome/installer/setup/setup.ico',
        ):
            write_changed(src / relative, icon_bytes)
        theme = src / 'chrome/app/theme/chromium'
        for path in [*theme.glob('product_logo_*.png'), *theme.glob('win/tiles/*.png')]:
            if 'mono' in path.name:
                continue
            with Image.open(path) as original:
                size = original.size
            output = io.BytesIO()
            artwork.resize(size, Image.Resampling.LANCZOS).save(output, format='PNG')
            write_changed(path, output.getvalue())
