"""Cargo build directory of scripts and tests: `CARGO_TARGET_DIR`, else `target`.

A script that passes `--target-dir` takes it from here. A literal `target`
inside the working copy would ignore the build directory the session set and
fill the checkout; with the variable unset the result is the usual
`<root>/target`, so CI keeps its layout.
"""
from __future__ import annotations

import os
from pathlib import Path


def cargo_target_dir(root: Path) -> Path:
    """The build directory Cargo itself would use for the workspace at `root`."""
    configured = os.environ.get("CARGO_TARGET_DIR")
    if configured:
        path = Path(configured)
        return path if path.is_absolute() else root / path
    return root / "target"
