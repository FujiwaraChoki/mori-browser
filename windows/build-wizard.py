"""Wrap the native Mori installer in a visible, branded setup wizard."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess

from PIL import Image

from branding import write_changed
from build_config import CHROMIUM_VERSION
from verify_branding import verify_branding


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version', default=CHROMIUM_VERSION)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    candidates = [Path(os.environ.get(key, '')) / suffix / 'ISCC.exe'
                  for key, suffix in [('ProgramFiles(x86)', 'Inno Setup 6'),
                                      ('LOCALAPPDATA', 'Programs/Inno Setup 6')]]
    compiler = next((path for path in candidates if path.is_file()), None)
    if compiler is None:
        raise RuntimeError('Install the Inno Setup compiler (JRSoftware.InnoSetup) first')
    native = repo / 'dist' / f'Mori-Setup-{args.version}-x64.exe'
    expected = native.with_suffix('.exe.sha256').read_text().split()[0]
    with native.open('rb') as source_file:
        if hashlib.file_digest(source_file, 'sha256').hexdigest() != expected:
            raise RuntimeError('Native installer checksum does not match')
    with Image.open(repo / 'mac/assets/AppIcon.png') as image:
        image = image.convert('RGBA')
        for name, size, icon_size, color in [
                ('wizard-large.bmp', (164, 314), (150, 150), '#242426'),
                ('wizard-small.bmp', (55, 55), (55, 55), '#ffffff')]:
            background = Image.new('RGB', size, color)
            icon = image.resize(icon_size, Image.Resampling.LANCZOS)
            background.paste(icon, ((size[0] - icon_size[0]) // 2,
                                    (size[1] - icon_size[1]) // 2), icon)
            import io
            data = io.BytesIO()
            background.save(data, 'BMP')
            write_changed(repo / 'windows/assets' / name, data.getvalue())
    subprocess.run([str(compiler), f'/DAppVersion={args.version}',
                    f'/DNativeInstaller={native}', str(repo / 'windows/installer.iss')], check=True)
    destination = repo / 'dist' / f'Mori-Setup-{args.version}-x64-Wizard.exe'
    verify_branding(repo, destination.parent, expected_version=args.version,
                    products=[(destination.name, 'Mori Installer')])
    with destination.open('rb') as installer_file:
        digest = hashlib.file_digest(installer_file, 'sha256').hexdigest()
    destination.with_suffix('.exe.sha256').write_text(f'{digest}  {destination.name}\n')
    print(destination)


if __name__ == '__main__':
    main()
