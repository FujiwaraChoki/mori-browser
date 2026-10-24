#!/usr/bin/env python3
"""Reject upstream checkouts that do not match Mori's shared patch baseline."""

import argparse
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, type=Path,
                        help="ungoogled-chromium core checkout")
    parser.add_argument("--source", type=Path, help="unpacked Chromium source")
    args = parser.parse_args()
    manifest = Path(__file__).resolve().parents[2] / "windows/chromium.json"
    expected = json.loads(manifest.read_text(encoding="utf-8"))["version"]
    try:
        versions = [(args.base / "chromium_version.txt",
                     (args.base / "chromium_version.txt").read_text().strip())]
        if args.source:
            version_file = args.source / "chrome/VERSION"
            fields = dict(line.strip().split("=", 1)
                          for line in version_file.read_text().splitlines()
                          if "=" in line)
            versions.append((version_file, ".".join(
                fields[key] for key in ("MAJOR", "MINOR", "BUILD", "PATCH"))))
        for path, actual in versions:
            if actual != expected:
                parser.exit(1, f"{path}: Chromium {actual}; Mori requires {expected}. "
                            "Prepare matching upstream sources before applying patches.\n")
    except (OSError, KeyError, ValueError) as error:
        parser.exit(1, f"Cannot verify Chromium version: {error}\n")
    print(f"Chromium baseline verified: {expected}")


if __name__ == "__main__":
    main()
