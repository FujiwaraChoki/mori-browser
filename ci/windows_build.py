"""Run trusted Windows jobs against this PC's persistent Chromium checkout."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

REPO = Path(__file__).resolve().parents[1]


def run(*args):
    subprocess.run([str(arg) for arg in args], cwd=REPO, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--operation', choices=['verify', 'build', 'package'], default='verify')
    parser.add_argument('--jobs', type=int, choices=range(1, 13), default=12)
    args = parser.parse_args()
    machine = json.loads(Path('C:/mori-ci/machine.json').read_text())
    os.environ['PATH'] = machine['depot_tools'] + os.pathsep + str(Path(sys.executable).parent) + os.pathsep + os.environ['PATH']
    checkout = Path(machine['checkout'])
    sys.path.insert(0, str(REPO / 'windows'))
    from build_config import CHROMIUM_VERSION, configure_toolchain, read_chromium_version, patch_content_hashes
    configure_toolchain(REPO)
    if read_chromium_version(checkout / 'src') != CHROMIUM_VERSION:
        raise RuntimeError('Prepare the pinned Chromium checkout before building')
    state = json.loads((checkout / 'mori-setup-state.json').read_text())
    for directory, prefix in [('linux/patches', 'mori'), ('windows/patches', 'windows')]:
        for path in (REPO / directory).glob('*.patch'):
            recorded = state.get(prefix + '/' + path.name)
            if recorded is not None and recorded not in patch_content_hashes(path.read_bytes())[1]:
                raise RuntimeError(f'{path.name} differs from the cached engine; prepare a fresh checkout')
    if shutil.disk_usage(checkout).free < 30 * 1024**3:
        raise RuntimeError('Windows builds need at least 30 GiB of free disk headroom')
    for tool in ('git', 'node', 'npm'):
        run('where.exe', tool)
    run(sys.executable, 'ci/checks.py')
    run(sys.executable, '-m', 'unittest', 'discover', '-s', 'windows/tests', '-v')
    if args.operation != 'verify':
        script = 'windows/package.py' if args.operation == 'package' else 'windows/build.py'
        run(sys.executable, script, '--checkout', checkout, '--jobs', args.jobs, '--relink-overlay')
        os.environ['MORI_TEST_BROWSER'] = str(checkout / 'src/out/Default/chrome.exe')
        run('cmd.exe', '/d', '/c', 'npm', 'ci', '--prefix', 'windows', '--ignore-scripts', '--no-audit', '--no-fund')
        run('cmd.exe', '/d', '/c', 'npm', '--prefix', 'windows', 'test')
    outputs = []
    if args.operation == 'package':
        run(sys.executable, 'windows/build-wizard.py')
        destination = Path(machine['artifacts']) / os.environ.get('GITHUB_RUN_ID', 'manual')
        destination.mkdir(parents=True, exist_ok=True)
        for output in (REPO / 'dist').glob('Mori-Setup-*'):
            if output.is_file():
                outputs.append(str(shutil.copy2(output, destination / output.name)))
    summary = f'Windows {args.operation} succeeded. Chromium {CHROMIUM_VERSION}.\n'
    summary += f'Persistent runtime: `{checkout / "src/out/Default/chrome.exe"}`\n'
    summary += '\n'.join(f'- Local artifact: `{path}`' for path in outputs)
    print(summary)
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a', encoding='utf-8') as handle:
            handle.write(summary + '\n')


if __name__ == '__main__':
    main()
