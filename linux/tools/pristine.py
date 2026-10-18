#!/usr/bin/env python3
"""Recreates the ungoogled-chromium version of Chromium files, before Mori's
patches: from the source tarball, plus ungoogled-chromium's and the portable
build's patches and domain substitution, for just the files given.

  linux/tools/pristine.py chrome/browser/ui/BUILD.gn ...

Writes them under ungoogled-chromium-linux/pristine/, where patch
regeneration (linux/tools/mkpatch.py) reads them.
"""
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
UC = ROOT / "ungoogled-chromium-linux"
PRISTINE = UC / "pristine"


def series(patch_dir):
    names = (patch_dir / "series").read_text().split()
    return [patch_dir / n for n in names if not n.startswith("#")]


def sections_for(patch_file, path):
    """The parts of `patch_file` that change `path`."""
    text = patch_file.read_text(errors="replace")
    parts = re.split(r"(?m)^(?=--- )", text)
    return [p for p in parts if re.search(rf"(?m)^\+\+\+ b/{re.escape(path)}(\s|$)", p)]


def main():
    files = sys.argv[1:]
    tarball = next((UC / "build" / "download_cache").glob("chromium-*.tar.xz"))
    top = tarball.name.removesuffix(".tar.xz").removesuffix("-lite")
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        subprocess.run(
            ["tar", "-xJf", str(tarball), "-C", str(tmp)] + [f"{top}/{f}" for f in files],
            check=True)
        tree = tmp / top
        patch_sets = series(UC / "ungoogled-chromium" / "patches") + series(UC / "patches")
        domsub = set((UC / "ungoogled-chromium" / "domain_substitution.list").read_text().split())
        regexes = []
        for line in (UC / "ungoogled-chromium" / "domain_regex.list").read_text().splitlines():
            if line.strip():
                pattern, replacement = line.split("#", 1)
                regexes.append((re.compile(pattern), replacement))
        for f in files:
            for patch_file in patch_sets:
                for section in sections_for(patch_file, f):
                    subprocess.run(["patch", "-p1", "-s", "-f", "-d", str(tree)],
                                   input=section, text=True, check=True)
            target = tree / f
            if f in domsub:
                text = target.read_text()
                for pattern, replacement in regexes:
                    text = pattern.sub(replacement, text)
                target.write_text(text)
            out = PRISTINE / f
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(target.read_text())
            print("pristine", f)


if __name__ == "__main__":
    main()
