"""The single version pin shared by Windows build and packaging tools."""
import json
from contextlib import contextmanager
import errno
import hashlib
import os
from pathlib import Path
import subprocess
import time
import uuid

CONFIG = json.loads(Path(__file__).with_name('chromium.json').read_text())
CHROMIUM_VERSION = CONFIG['version']
DEFAULT_CHECKOUT = CONFIG['default_checkout']


def patch_content_hashes(data):
    """Canonical hash plus compatible legacy hashes for LF/CRLF-only changes."""
    normalized = data.replace(b'\r\n', b'\n')
    canonical = hashlib.sha256(normalized).hexdigest()
    compatible = {canonical, hashlib.sha256(data).hexdigest(),
                  hashlib.sha256(normalized.replace(b'\n', b'\r\n')).hexdigest()}
    return canonical, compatible


def link_mori_sources(repo, source, allow_relink=False):
    """Bind this source revision to Chromium, preserving any previous junction."""
    target = (Path(repo) / 'linux/mori').resolve()
    overlay = Path(source) / 'chrome/browser/ui/views/mori'
    if overlay.exists() and overlay.resolve() == target:
        return
    if overlay.exists() or overlay.is_symlink() or overlay.is_junction():
        if not allow_relink:
            raise RuntimeError(f'{overlay} already exists and does not point to Mori sources')
        if not overlay.is_junction():
            raise RuntimeError(f'Refusing to move a non-junction source folder: {overlay}')
        previous = overlay.resolve()
        for name in ('mori_sidebar.cc', 'mori_toolbar_layout.cc'):
            if not (previous / name).is_file():
                raise RuntimeError(f'{previous} does not look like Mori source')
        source = Path(source).resolve()
        archive = source.parent / 'mori-overlay-links'
        archive.mkdir(exist_ok=True)
        if not overlay.parent.resolve().is_relative_to(source):
            raise RuntimeError('Overlay parent escapes the Chromium checkout')
        if archive.resolve().parent != source.parent:
            raise RuntimeError('Junction archive escapes the Chromium checkout')
        saved = archive / uuid.uuid4().hex
        # Rename the junction itself, never move/delete its target or contents.
        overlay.rename(saved)
        print(f'Preserved previous source junction at {saved}', flush=True)
    overlay.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['cmd.exe', '/d', '/c', 'mklink', '/J', str(overlay), str(target)],
                   check=True, capture_output=True)


def configure_toolchain(repo):
    os.environ['DEPOT_TOOLS_WIN_TOOLCHAIN'] = '0'
    os.environ['PATH'] = str(Path(repo) / 'depot_tools') + os.pathsep + os.environ['PATH']
    vswhere = Path(os.environ['ProgramFiles(x86)']) / 'Microsoft Visual Studio/Installer/vswhere.exe'
    installations = json.loads(subprocess.check_output(
        [str(vswhere), '-latest', '-products', '*', '-requires',
         'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-format', 'json'], text=True))
    if not installations:
        raise RuntimeError('Visual Studio C++ Build Tools were not found')
    installation = installations[0]
    year = {'18': '2026', '17': '2022'}[installation['installationVersion'].split('.')[0]]
    os.environ[f'vs{year}_install'] = installation['installationPath']


def read_chromium_version(source):
    values = dict(line.split('=', 1) for line in
                  (Path(source) / 'chrome/VERSION').read_text().splitlines())
    return '.'.join(values[key] for key in ('MAJOR', 'MINOR', 'BUILD', 'PATCH'))


def _active_checkout_ninjas(checkout):
    ninja = checkout / 'src/third_party/ninja/ninja.exe'
    if not ninja.exists():
        return []
    command = ("$ErrorActionPreference='Stop'; "
               "Get-CimInstance Win32_Process -Filter \"Name = 'ninja.exe'\" | "
               "Select-Object ProcessId,ExecutablePath | ConvertTo-Json -Compress")
    output = subprocess.check_output(
        ['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', command],
        text=True, timeout=30, creationflags=subprocess.CREATE_NO_WINDOW)
    processes = (json.loads(output) if output.strip() else []) or []
    if isinstance(processes, dict):
        processes = [processes]
    expected = str(ninja).casefold()
    return sorted(process['ProcessId'] for process in processes
                  if (process.get('ExecutablePath') or '').casefold() == expected)


@contextmanager
def checkout_build_lock(checkout, poll_interval=5):
    """Serialize setup and compilation, including older builds without a lock."""
    import msvcrt

    checkout = Path(checkout).resolve()
    checkout.mkdir(parents=True, exist_ok=True)
    with (checkout / '.mori-build.lock').open('a+b') as handle:
        if handle.seek(0, os.SEEK_END) == 0:
            handle.write(b'\0')
            handle.flush()
        reported = False
        while True:
            handle.seek(0)
            try:
                msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
                break
            except OSError as error:
                if error.errno not in (errno.EACCES, errno.EAGAIN, errno.EDEADLK):
                    raise
                if not reported:
                    print('Waiting for another Mori build to release the checkout...', flush=True)
                    reported = True
                time.sleep(poll_interval)
        try:
            # Builds started before this lock was added still need to finish.
            last_waiting = None
            while waiting := _active_checkout_ninjas(checkout):
                if waiting != last_waiting:
                    print(f'Waiting for existing Ninja processes: {waiting}', flush=True)
                    last_waiting = waiting
                time.sleep(poll_interval)
            yield
        finally:
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
