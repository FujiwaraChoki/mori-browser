"""Persistent native Linux Chromium builds, including on the Mori WSL runner.

Unlike linux/build.sh, this entry point uses gclient and the core pins directly;
it does not depend on a portablelinux packaging release being available yet.
"""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import uuid

REPO = Path(__file__).resolve().parents[1]
CONFIG = json.loads((REPO / 'windows/chromium.json').read_text())
TOOLS = json.loads((REPO / 'ci/linux.json').read_text())
sys.path.insert(0, str(REPO / 'windows'))
from build_config import patch_content_hashes


def run(*args, cwd=None, **kwargs):
    return subprocess.run([str(arg) for arg in args], cwd=cwd, check=True, **kwargs)


def git_revision(path):
    return subprocess.check_output(['git', '-C', str(path), 'rev-parse', 'HEAD'], text=True).strip()


def ensure_repository(path, url, revision):
    if not (path / '.git').exists():
        if path.exists() and any(path.iterdir()):
            raise RuntimeError(f'Refusing to initialize a nonempty directory: {path}')
        path.mkdir(parents=True, exist_ok=True)
        run('git', 'init', '--quiet', path)
        run('git', '-C', path, 'remote', 'add', 'origin', url)
    present = subprocess.run(['git', '-C', str(path), 'cat-file', '-e', revision + '^{commit}'],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if present.returncode:
        run('git', '-C', path, 'fetch', '--depth', '1', '--no-tags', 'origin', revision)
    head = subprocess.run(['git', '-C', str(path), 'rev-parse', '--verify', 'HEAD'],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if head.returncode:
        run('git', '-C', path, 'checkout', '--detach', revision)
    if git_revision(path) != revision:
        raise RuntimeError(f'{path} revision differs; prepare a new versioned checkout')


@contextmanager
def build_lock(checkout):
    import fcntl
    checkout.mkdir(parents=True, exist_ok=True)
    with (checkout / '.mori-ci.lock').open('a') as handle:
        print(f'Waiting for Linux checkout lock: {checkout}', flush=True)
        fcntl.flock(handle, fcntl.LOCK_EX)
        yield


def disk_headroom(checkout, gib):
    mounts = [checkout]
    if Path('/mnt/c').is_mount():
        mounts.append(Path('/mnt/c'))  # WSL's virtual free space can exceed the host disk.
    if min(shutil.disk_usage(path).free for path in mounts) < gib * 1024**3:
        raise RuntimeError(f'Linux build requires at least {gib} GiB free on both guest and host')


def prepare(checkout, state, save):
    source, base = checkout / 'src', checkout / 'ungoogled-base'
    depot = checkout / 'depot_tools'
    ensure_repository(depot, 'https://chromium.googlesource.com/chromium/tools/depot_tools.git',
                      TOOLS['depot_tools_revision'])
    os.environ.update(DEPOT_TOOLS_UPDATE='0', DEPOT_TOOLS_METRICS='0',
                      GIT_TERMINAL_PROMPT='0')
    os.environ['PATH'] = str(depot) + os.pathsep + os.environ['PATH']
    if not state.get('synced'):
        disk_headroom(checkout, 180)
        ensure_repository(source, 'https://chromium.googlesource.com/chromium/src.git', CONFIG['revision'])
        gclient = checkout / '.gclient'
        if not gclient.exists():
            gclient.write_text('solutions = ' + repr([{
                'name': 'src', 'url': 'https://chromium.googlesource.com/chromium/src.git',
                'managed': False, 'custom_deps': {}, 'custom_vars': {},
            }]) + "\ntarget_os = ['linux']\n")
        run(depot / 'gclient', 'sync', '--nohooks', '--no-history', '--jobs', '12',
            '--revision', 'src@' + CONFIG['revision'], cwd=checkout)
        run(depot / 'gclient', 'runhooks', cwd=checkout)
        state['synced'] = True
        save()
    if git_revision(source) != CONFIG['revision']:
        raise RuntimeError('Chromium source revision does not match the manifest')
    ensure_repository(base, 'https://github.com/ungoogled-software/ungoogled-chromium.git',
                      CONFIG['ungoogled_revision'])
    run(sys.executable, REPO / 'linux/tools/check_chromium_version.py', '--base', base, '--source', source)

    def patch(path, key):
        digest, compatible = patch_content_hashes(path.read_bytes())
        if key in state:
            if state[key] not in compatible:
                raise RuntimeError(f'{key} changed after application; use a fresh checkout')
            if state[key] != digest:
                state[key] = digest
                save()
            return
        command = ['patch', '-d', str(source), '-p1', '--batch', '--forward',
                   '--ignore-whitespace', '--no-backup-if-mismatch', '-i', str(path)]
        run(*command, '--dry-run', stdout=subprocess.DEVNULL)
        run(*command)
        state[key] = digest
        save()

    for line in (base / 'patches/series').read_text().splitlines():
        if line.strip() and not line.lstrip().startswith('#'):
            patch(base / 'patches' / line.strip(), 'ungoogled/' + line.strip())
    if not state.get('domains'):
        run(sys.executable, base / 'utils/domain_substitution.py', 'apply',
            '-r', base / 'domain_regex.list', '-f', base / 'domain_substitution.list', source)
        state['domains'] = True
        save()
    overlay = source / 'chrome/browser/ui/views/mori'
    target = REPO / 'linux/mori'
    if overlay.is_symlink() and overlay.resolve() != target.resolve():
        archive = checkout / 'mori-overlay-links'
        archive.mkdir(exist_ok=True)
        overlay.rename(archive / uuid.uuid4().hex)
    if not overlay.exists() and not overlay.is_symlink():
        overlay.symlink_to(target, target_is_directory=True)
    if not overlay.is_symlink() or overlay.resolve() != target.resolve():
        raise RuntimeError('Refusing to replace a real or unexpected Mori source directory')
    for path in sorted((REPO / 'linux/patches').glob('*.patch')):
        patch(path, 'mori/' + path.name)
    out = source / 'out/Default'
    out.mkdir(parents=True, exist_ok=True)
    flags = (base / 'flags.gn').read_text() + '''
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
use_sysroot=true
use_mold=false
proprietary_codecs=true
ffmpeg_branding="Chrome"
'''
    args = out / 'args.gn'
    if not args.exists() or args.read_text() != flags:
        args.write_text(flags)
    run(source / 'buildtools/linux64/gn', 'gen', 'out/Default', '--fail-on-unused-args', cwd=source)
    return source, out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--operation', choices=['verify', 'build', 'package'], default='verify')
    parser.add_argument('--jobs', type=int, choices=range(1, 13), default=12)
    parser.add_argument('--checkout', type=Path,
                        default=Path.home() / '.local/share/mori-ci' / ('chromium-' + CONFIG['version']))
    args = parser.parse_args()
    if sys.platform != 'linux':
        parser.error('Run inside native Linux or the Mori-Linux WSL distribution')
    if os.geteuid() == 0:
        parser.error('The runner must run as an ordinary user')
    args.checkout.mkdir(parents=True, exist_ok=True)
    for tool in ('git', 'node', 'ninja', 'patch', 'c++', 'pkg-config', 'flock'):
        if not shutil.which(tool):
            raise RuntimeError(f'Missing Linux build prerequisite: {tool}')
    disk_headroom(args.checkout, 40)
    run(sys.executable, REPO / 'ci/checks.py')
    run(sys.executable, '-m', 'unittest', 'discover', '-s', REPO / 'windows/tests', '-v')
    smoke = args.checkout / 'mori-markdown-test'
    run('c++', '-std=c++20', '-Wall', '-Wextra', '-pedantic', REPO / 'linux/tests/mori_markdown_test.cc', '-o', smoke)
    run(smoke)
    outputs = []
    with build_lock(args.checkout):
        if args.operation != 'verify':
            state_file = args.checkout / 'mori-ci-state.json'
            state = json.loads(state_file.read_text()) if state_file.exists() else {}
            def save():
                temporary = state_file.with_suffix('.json.tmp')
                temporary.write_text(json.dumps(state, indent=2) + '\n')
                temporary.replace(state_file)
            source, out = prepare(args.checkout, state, save)
            disk_headroom(args.checkout, 40)
            run(source / 'third_party/ninja/ninja', '-C', out, '-j', args.jobs, 'chrome', cwd=source)
            run(out / 'chrome', '--version')
            if args.operation == 'package':
                destination = Path.home() / 'mori-artifacts' / os.environ.get('GITHUB_RUN_ID', 'manual')
                destination.mkdir(parents=True, exist_ok=True)
                runtime = out / 'mori-runtime-deps.txt'
                with runtime.open('w') as handle:
                    run(source / 'buildtools/linux64/gn', 'desc', out, '//chrome:chrome', 'runtime_deps',
                        cwd=source, stdout=handle)
                archive = destination / f'Mori-{CONFIG["version"]}-linux-x64.tar.xz'
                entries = [line.strip() for line in runtime.read_text().splitlines() if line.strip()]
                # Keep resources relative to out/Default; never archive the source checkout.
                if any(Path(name).is_absolute() or '..' in Path(name).parts for name in entries):
                    raise RuntimeError('Unexpected runtime dependency outside the output directory')
                import tarfile
                with tarfile.open(archive, 'w:xz') as bundle:
                    for name in dict.fromkeys(entries):
                        bundle.add(out / name, arcname='Mori/' + name)
                with archive.open('rb') as handle:
                    digest = hashlib.file_digest(handle, 'sha256').hexdigest()
                archive.with_suffix('.tar.xz.sha256').write_text(f'{digest}  {archive.name}\n')
                outputs.append(str(archive))
    summary = f'Linux {args.operation} succeeded. Chromium {CONFIG["version"]}.\n'
    summary += f'Persistent checkout: `{args.checkout}`\n'
    if args.operation == 'verify':
        summary += 'Runner/toolchain smoke verified; full Chromium compilation was not requested.\n'
    summary += '\n'.join(f'- Local artifact: `{path}`' for path in outputs)
    print(summary)
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as handle:
            handle.write(summary + '\n')


if __name__ == '__main__':
    main()
