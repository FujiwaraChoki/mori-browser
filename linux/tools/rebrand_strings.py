#!/usr/bin/env python3
"""Names the browser Mori in Chromium's user-facing strings.

Rewrites "Chromium" and "Chrome" to "Mori" in the text of the <message>s
of Chromium's string files (not in descriptions, IDs or comments). Names
of other things stay: ChromiumOS, "The Chromium Authors", credits to the
Chromium open source project, Google Chrome, the Chrome Web Store, Chrome
apps and extensions. Files for ChromeOS, Google's Gemini features and
address-bar keywords are left alone.

  linux/tools/rebrand_strings.py <src dir> [--list]

Run on pristine files: it is what produces patch 0023. --list prints the
files it would change (for pristine.py and mkpatch.py).
"""
import pathlib
import re
import sys

GLOBS = [
    "chrome/app/*.grd",
    "chrome/app/*.grdp",
    "components/*strings*.grd",
    "components/*strings*.grdp",
]
SKIP = [
    "google_chrome",
    "omnibox_pedal_ui_strings",
    "os_settings_strings",
    "chromeos_strings",
    "glic_strings",
    "actor_strings",
    "skills_strings",
]

MESSAGE = re.compile(r"(<message\b[^>]*>)(.*?)(</message>)", re.S)
NAME = re.compile(
    r"(?<!Google )\b(?:Chromium|Chrome)\b"
    r"(?!OS| OS| Authors| open source| project| Web Store| [Aa]pps?\b"
    r"| [Ee]xtensions?\b| Enterprise| for Testing| Browser Cloud"
    r'|<ph name="END_LINK_CHROMIUM")')


def rebrand(text):
    return MESSAGE.sub(
        lambda m: m.group(1) + NAME.sub("Mori", m.group(2)) + m.group(3),
        text)


def files(src):
    out = set()
    for pattern in GLOBS:
        for path in src.glob(pattern):
            name = str(path.relative_to(src))
            if any(skip in name for skip in SKIP):
                continue
            text = path.read_text()
            if rebrand(text) != text:
                out.add(name)
    return sorted(out)


def main():
    src = pathlib.Path(sys.argv[1])
    if "--list" in sys.argv:
        print("\n".join(files(src)))
        return
    for name in files(src):
        path = src / name
        before = path.read_text()
        after = rebrand(before)
        path.write_text(after)
        renamed = len(re.findall(r"\bChrom(?:e|ium)\b", before)) - len(
            re.findall(r"\bChrom(?:e|ium)\b", after))
        print(f"{name}: {renamed}")


if __name__ == "__main__":
    main()
