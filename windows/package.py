"""Build a branded native Windows installer and publish its checksum locally."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

from verify_branding import verify_branding
from build import build as build_checkout
from build_config import CONFIG, CHROMIUM_VERSION, DEFAULT_CHECKOUT, checkout_build_lock, read_chromium_version


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checkout', default=DEFAULT_CHECKOUT)
    parser.add_argument('--jobs', type=int, default=12)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--relink-overlay', action='store_true')
    args = parser.parse_args()
    with checkout_build_lock(Path(args.checkout)):
        package(args)


def package(args):
    repo = Path(__file__).resolve().parents[1]
    checkout = Path(args.checkout).absolute()
    if not args.skip_build:
        build_checkout(argparse.Namespace(
            checkout=str(checkout), base=None, setup_only=False,
            jobs=args.jobs, target='mini_installer',
            relink_overlay=getattr(args, 'relink_overlay', False)))
    version = read_chromium_version(checkout / 'src')
    if version != CHROMIUM_VERSION:
        raise RuntimeError(f'Expected Chromium {CHROMIUM_VERSION}, found {version}')
    revision = subprocess.check_output(
        ['git', '-C', str(checkout / 'src'), 'rev-parse', 'HEAD'], text=True).strip()
    if revision != CONFIG['revision']:
        raise RuntimeError('The source revision does not match windows/chromium.json')
    installer = checkout / 'src/out/Default/mini_installer.exe'
    if not installer.is_file():
        raise RuntimeError('mini_installer.exe has not been built')
    verify_branding(repo, installer.parent, expected_version=version)
    destination = repo / 'dist' / f'Mori-Setup-{version}-x64.exe'
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(installer, destination)
    with destination.open('rb') as installer_file:
        digest = hashlib.file_digest(installer_file, 'sha256').hexdigest()
    destination.with_suffix('.exe.sha256').write_text(f'{digest}  {destination.name}\n')
    metadata = {'name': 'Mori', 'version': version, 'architecture': 'x64',
                'chromium_revision': revision,
                'ungoogled_release': CONFIG['ungoogled_release'],
                'installer': destination.name, 'sha256': digest,
                'source': 'https://github.com/FujiwaraChoki/mori-browser',
                'icon_source': 'mac/assets/AppIcon.png', 'signed': False}
    destination.with_suffix('.exe.json').write_text(json.dumps(metadata, indent=2) + '\n')
    print(destination)
    print(f'SHA256: {digest}')


if __name__ == '__main__':
    main()
