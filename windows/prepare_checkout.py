"""Prepare a separate Chromium checkout, reusing existing local Git objects.

The cache checkout is read-only. Its object stores must remain available while
this checkout uses them, so do not delete the cache checkout after upgrading.
"""
import argparse
import ast
from concurrent.futures import ThreadPoolExecutor, as_completed
import os
from pathlib import Path
import shutil
import subprocess

from build_config import CONFIG, DEFAULT_CHECKOUT, configure_toolchain


def run(*args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def seed_repository(source, destination, url):
    if (destination / '.git/mori-cache-ready').exists():
        return
    git_dir = Path(subprocess.check_output(
        ['git', '-C', str(source), 'rev-parse', '--absolute-git-dir'], text=True).strip())
    revision = subprocess.check_output(
        ['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
    run('git', 'init', '--quiet', destination)
    run('git', '-C', destination, 'config', 'core.autocrlf', 'false')
    run('git', '-C', destination, 'config', 'core.longpaths', 'true')
    run('git', '-C', destination, 'config', 'checkout.workers', '8')
    run('git', '-C', destination, 'config', 'fetch.recurseSubmodules', 'false')
    run('git', '-C', destination, 'config', 'remote.origin.url', url)
    run('git', '-C', destination, 'config', 'remote.origin.fetch',
        '+refs/heads/*:refs/remotes/origin/*')
    alternates = destination / '.git/objects/info/alternates'
    alternates.parent.mkdir(parents=True, exist_ok=True)
    alternates.write_text((git_dir / 'objects').as_posix() + '\n', newline='\n')
    if (git_dir / 'shallow').is_file():
        shutil.copy2(git_dir / 'shallow', destination / '.git/shallow')
    run('git', '-C', destination, 'update-ref', 'refs/remotes/mori-cache/HEAD', revision)
    run('git', '-C', destination, 'update-ref', '--no-deref', 'HEAD', revision)
    (destination / '.git/mori-cache-ready').write_text(revision + '\n', newline='\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checkout', default=DEFAULT_CHECKOUT)
    parser.add_argument('--cache-checkout', default=r'C:\mori-build')
    parser.add_argument('--jobs', type=int, default=12)
    parser.add_argument('--no-sync', action='store_true')
    parser.add_argument('--sync-only', action='store_true')
    parser.add_argument('--hooks-only', action='store_true')
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    cache = Path(args.cache_checkout).resolve()
    checkout = Path(args.checkout).resolve()
    if cache == checkout:
        raise RuntimeError('Use a separate checkout to preserve the existing build')
    checkout.mkdir(parents=True, exist_ok=True)
    os.environ['DEPOT_TOOLS_WIN_TOOLCHAIN'] = '0'
    os.environ['DEPOT_TOOLS_UPDATE'] = '0'
    os.environ['GIT_TERMINAL_PROMPT'] = '0'
    os.environ['PATH'] = str(repo / 'depot_tools') + os.pathsep + os.environ['PATH']
    configure_toolchain(repo)

    def synchronize():
        if (checkout / 'mori-setup-state.json').exists():
            raise RuntimeError('Dependency syncing must precede applying Mori patches')
        run('cmd.exe', '/d', '/c', repo / 'depot_tools/gclient.bat', 'sync',
            '--nohooks', '--no-history', '--force', '-j', args.jobs,
            '--revision', f'src@{CONFIG["revision"]}', cwd=checkout)
        run_hooks()

    def run_hooks():
        run('cmd.exe', '/d', '/c', repo / 'depot_tools/gclient.bat', 'runhooks', cwd=checkout)

    if args.hooks_only:
        run_hooks()
        return
    if args.sync_only:
        synchronize()
        return

    source = checkout / 'src'
    seed_repository(cache / 'src', source,
                    'https://chromium.googlesource.com/chromium/src.git')
    revision = CONFIG['revision']
    available = subprocess.run(['git', '-C', str(source), 'cat-file', '-e', revision + '^{tree}'],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if available.returncode:
        print(f'Fetching Chromium {CONFIG["version"]}', flush=True)
        run('git', '-C', source, '-c', 'protocol.version=2', 'fetch',
            '--no-tags', '--depth', '1', 'origin', revision)
    # Never check out again after patches have been applied.
    if not (checkout / 'mori-setup-state.json').exists():
        run('git', '-C', source, '-c', 'checkout.workers=8',
            'checkout', '--force', '--detach', revision)

    base_cache = repo / 'ungoogled-chromium-linux/ungoogled-chromium'
    seed_repository(base_cache, checkout / 'ungoogled-base',
                    'https://github.com/ungoogled-software/ungoogled-chromium.git')
    base = checkout / 'ungoogled-base'
    core_revision = CONFIG['ungoogled_revision']
    available = subprocess.run(['git', '-C', str(base), 'cat-file', '-e',
                                core_revision + '^{tree}'],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if available.returncode:
        run('git', '-C', base, 'fetch', '--no-tags', '--depth', '1', 'origin', core_revision)
    run('git', '-C', base, 'checkout', '--force', '--detach', core_revision)

    tree = ast.parse((cache / '.gclient_entries').read_text())
    entries = ast.literal_eval(tree.body[0].value)
    candidates = [(name, url) for name, url in entries.items()
                  if name != 'src' and ':' not in name and isinstance(url, str)
                  and url.startswith('https://') and (cache / name / '.git').exists()]
    def prepare(item):
        name, url = item
        seed_repository(cache / name, checkout / name, url.split('@')[0])
        return name
    print(f'Reusing {len(candidates)} local dependency object stores', flush=True)
    with ThreadPoolExecutor(max_workers=4) as pool:
        for count, future in enumerate(as_completed(
                [pool.submit(prepare, item) for item in candidates]), 1):
            name = future.result()
            if count % 20 == 0 or count == len(candidates):
                print(f'Prepared {count}/{len(candidates)}: {name}', flush=True)
    if not (checkout / '.gclient').exists():
        shutil.copy2(cache / '.gclient', checkout / '.gclient')
    if not args.no_sync:
        synchronize()


if __name__ == '__main__':
    main()
