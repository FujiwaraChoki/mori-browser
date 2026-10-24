"""Prepare and build Mori with a full, synced Chromium Windows checkout."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

from branding import apply_branding
from build_config import CONFIG, CHROMIUM_VERSION, DEFAULT_CHECKOUT, checkout_build_lock, configure_toolchain, read_chromium_version


def run(*args, **kwargs):
    subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checkout', default=DEFAULT_CHECKOUT)
    parser.add_argument('--base', help='Override the matching ungoogled-chromium patch repository')
    parser.add_argument('--setup-only', action='store_true')
    parser.add_argument('--jobs', type=int, default=12)
    parser.add_argument('--target', choices=['chrome', 'mini_installer'], default='chrome')
    args = parser.parse_args()
    with checkout_build_lock(Path(args.checkout)):
        build(args)


def build(args):
    repo = Path(__file__).resolve().parents[1]
    checkout = Path(args.checkout).absolute()
    src = checkout / 'src'
    base = Path(args.base) if args.base else checkout / 'ungoogled-base'
    if not base.exists() and not args.base:
        base = repo / 'ungoogled-chromium-linux/ungoogled-chromium'
    patch_bin = Path(os.environ.get('ProgramFiles', r'C:\Program Files')) / 'Git/usr/bin/patch.exe'
    if read_chromium_version(src) != CHROMIUM_VERSION:
        raise RuntimeError(f'Expected a synced Chromium {CHROMIUM_VERSION} checkout')
    if (base / 'chromium_version.txt').read_text().strip() != CHROMIUM_VERSION:
        raise RuntimeError('The ungoogled patch repository does not match Chromium')
    for path, revision in [(src, CONFIG['revision']), (base, CONFIG['ungoogled_revision'])]:
        actual = subprocess.check_output(['git', '-C', str(path), 'rev-parse', 'HEAD'],
                                         text=True).strip()
        if actual != revision:
            raise RuntimeError(f'{path} is at {actual}, expected {revision}')
    configure_toolchain(repo)
    state_file = checkout / 'mori-setup-state.json'
    state = json.loads(state_file.read_text()) if state_file.exists() else {}

    def save():
        temporary = state_file.with_suffix('.json.tmp')
        temporary.write_text(json.dumps(state, indent=2) + '\n')
        temporary.replace(state_file)

    def apply_patch(path, key):
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if key in state:
            if state[key] != digest:
                raise RuntimeError(f'{path.name} changed after application; use a fresh checkout')
            return
        command = [str(patch_bin), '-d', str(src), '-p1', '--batch', '--forward',
                   '--ignore-whitespace', '--no-backup-if-mismatch', '-i', str(path)]
        # Validate every hunk before modifying any file in this patch.
        run(*command, '--dry-run', stdout=subprocess.DEVNULL)
        print(f'Applying {key}', flush=True)
        run(*command)
        state[key] = digest
        save()

    for line in (base / 'patches/series').read_text().splitlines():
        line = line.strip()
        if line and not line.startswith('#'):
            apply_patch(base / 'patches' / line, 'ungoogled/' + line)

    if not state.get('domain-substitution'):
        run(sys.executable, base / 'utils/domain_substitution.py', 'apply',
            '-r', base / 'domain_regex.list', '-f', base / 'domain_substitution.list', src)
        state['domain-substitution'] = True
        save()

    overlay = src / 'chrome/browser/ui/views/mori'
    if not overlay.exists():
        run('cmd.exe', '/c', 'mklink', '/J', str(overlay), str(repo / 'linux/mori'))
    elif overlay.resolve() != (repo / 'linux/mori').resolve():
        raise RuntimeError(f'{overlay} already exists and does not point to Mori sources')
    for patch in sorted((repo / 'linux/patches').glob('*.patch')):
        apply_patch(patch, 'mori/' + patch.name)
    for patch in sorted((repo / 'windows/patches').glob('*.patch')):
        apply_patch(patch, 'windows/' + patch.name)
    apply_branding(repo, src)

    out = src / 'out/Default'
    out.mkdir(parents=True, exist_ok=True)
    flags = (base / 'flags.gn').read_text()
    flags += '''
# Mori Windows development build: bundled Chromium toolchains, local execution.
# Enable H.264/AAC for MP4 and MediaSource playback (e.g. X videos).
proprietary_codecs=true
ffmpeg_branding="Chrome"
is_official_build=false
is_debug=false
is_component_build=true
symbol_level=0
blink_symbol_level=0
v8_symbol_level=0
dcheck_always_on=false
use_thin_lto=false
target_cpu="x64"
use_remoteexec=false
use_siso=false
'''
    gn_args = out / 'args.gn'
    if not gn_args.exists() or gn_args.read_text() != flags:
        gn_args.write_text(flags)
    run(src / 'buildtools/win/gn.exe', 'gen', 'out/Default', '--fail-on-unused-args', cwd=src)
    if not args.setup_only:
        run(src / 'third_party/ninja/ninja.exe',
            '-C', 'out/Default', '-j', args.jobs, args.target, cwd=src)


if __name__ == '__main__':
    main()
