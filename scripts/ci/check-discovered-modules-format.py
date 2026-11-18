#!/usr/bin/env python3
"""Check all tracked Rust sources in crates, including path and generated modules.

Each source is passed directly to rustfmt. Disabling child traversal makes the
file set independent of cfg, #[path], and module lists generated in OUT_DIR.
"""
from __future__ import annotations

from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
EDITION = "2021"
# Bound the argument list for Windows.
BATCH_SIZE = 64


def tracked_sources(root: Path) -> list[str]:
    listing = subprocess.run(
        ["git", "ls-files", "-z", "crates/*.rs"], cwd=root, check=True, capture_output=True
    ).stdout
    return sorted(name for name in listing.decode("utf-8").split("\0") if name)


def main() -> int:
    sources = tracked_sources(ROOT)
    if not sources:
        print("error: no tracked Rust sources were found in crates", file=sys.stderr)
        return 1
    failed = False
    for start in range(0, len(sources), BATCH_SIZE):
        result = subprocess.run(
            ["rustfmt", "--edition", EDITION, "--config", "skip_children=true", "--check",
             *sources[start:start + BATCH_SIZE]], cwd=ROOT
        )
        failed |= result.returncode != 0
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
