#!/usr/bin/env python3
"""Check the format of the Rust modules that build.rs declares.

`cargo fmt` follows `mod` items from each crate root and does not see the ones
a build script writes into `OUT_DIR`. A file that includes such a list declares
every `*.rs` of one directory: `lib.rs` of its own, any other file of the
directory named after it. This pass gives those files to `rustfmt` directly.
"""
from __future__ import annotations

from pathlib import Path, PurePosixPath
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
INCLUDE = re.compile(r'include!\s*\(\s*concat!\s*\(\s*env!\s*\(\s*"OUT_DIR"\s*\)\s*,\s*"/[a-z0-9_]+_modules\.rs"')
ROOT_FILE = "lib.rs"
EDITION = "2021"


def tracked_sources(root: Path) -> list[str]:
    listing = subprocess.run(
        ["git", "ls-files", "-z", "crates/*.rs"], cwd=root, check=True, capture_output=True
    ).stdout
    return [name for name in listing.decode("utf-8").split("\0") if name]


def discovered_modules(root: Path, sources: list[str]) -> list[str]:
    directories = set()
    for name in sources:
        if INCLUDE.search((root / name).read_text(encoding="utf-8", errors="replace")):
            path = PurePosixPath(name)
            directories.add(path.parent if path.name == ROOT_FILE else path.with_suffix(""))
    return sorted(
        name
        for name in sources
        if PurePosixPath(name).parent in directories and PurePosixPath(name).name != ROOT_FILE
    )


def main() -> int:
    modules = discovered_modules(ROOT, tracked_sources(ROOT))
    if not modules:
        print("no modules declared by build.rs were found", file=sys.stderr)
        return 1
    return subprocess.run(["rustfmt", "--edition", EDITION, "--check", *modules], cwd=ROOT).returncode


if __name__ == "__main__":
    raise SystemExit(main())
