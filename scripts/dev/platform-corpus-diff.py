#!/usr/bin/env python3
"""Compare two platform XML corpus directories.

A corpus directory holds the Designer export of an empty configuration and an
empty extension made by one platform build, for example
`tests/fixtures/formats/platform_xml/2.20`. The report lists files present on
one side only, files whose bytes differ, and the root `version` attribute of
every XML file on both sides. With `--show` it prints a unified diff of each
differing file with the root `version` value masked, so the remaining lines
are the changes in content between the two export formats.

The exit code is 0 when both sides have the same files and bytes, 1 when they
differ and 2 on a usage or read error.
"""

from __future__ import annotations

import argparse
import difflib
import re
import sys
from pathlib import Path
from typing import Dict, List, Optional, Sequence

ROOT_VERSION = re.compile(rb"<MetaDataObject\b[^>]*?\sversion=\"([^\"]*)\"", re.S)


def corpus_files(root: Path) -> Dict[str, Path]:
    return {
        path.relative_to(root).as_posix(): path
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


def root_version(data: bytes) -> Optional[str]:
    match = ROOT_VERSION.search(data)
    return match.group(1).decode("utf-8") if match else None


def masked_lines(data: bytes) -> List[str]:
    match = ROOT_VERSION.search(data)
    masked = data
    if match:
        masked = data[: match.start(1)] + b"*" + data[match.end(1) :]
    text = masked.decode("utf-8-sig", errors="replace")
    return text.replace("\r\n", "\n").splitlines(keepends=True)


def compare(left: Path, right: Path, show: bool, out) -> int:
    left_files = corpus_files(left)
    right_files = corpus_files(right)
    differs = False
    for name in sorted(set(left_files) | set(right_files)):
        if name not in right_files:
            print(f"only in {left}: {name}", file=out)
            differs = True
            continue
        if name not in left_files:
            print(f"only in {right}: {name}", file=out)
            differs = True
            continue
        a = left_files[name].read_bytes()
        b = right_files[name].read_bytes()
        versions = ""
        if name.endswith(".xml"):
            versions = f"  version {root_version(a)} -> {root_version(b)}"
        if a == b:
            print(f"same     {name}{versions}", file=out)
            continue
        differs = True
        print(f"differs  {name}{versions}", file=out)
        if show:
            out.writelines(
                difflib.unified_diff(
                    masked_lines(a),
                    masked_lines(b),
                    fromfile=f"{left.name}/{name}",
                    tofile=f"{right.name}/{name}",
                )
            )
    return 1 if differs else 0


def main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("left", type=Path, help="first corpus directory")
    parser.add_argument("right", type=Path, help="second corpus directory")
    parser.add_argument(
        "--show",
        action="store_true",
        help="print a unified diff of differing files, root version masked",
    )
    args = parser.parse_args(argv)
    for side in (args.left, args.right):
        if not side.is_dir():
            print(f"not a directory: {side}", file=sys.stderr)
            return 2
    try:
        return compare(args.left, args.right, args.show, sys.stdout)
    except OSError as error:
        print(f"read error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.exit(main(sys.argv[1:]))
