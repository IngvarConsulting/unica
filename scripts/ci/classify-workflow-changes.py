#!/usr/bin/env python3
"""Classify changed paths for the Unica GitHub Actions workflow."""

from __future__ import annotations

import sys
from collections.abc import Iterable
from typing import TextIO


RELEASE_ARTIFACT_PATHS = {
    ".agents/plugins/marketplace.json",
    ".github/workflows/unica-plugin-release.yml",
    "Cargo.toml",
    "Cargo.lock",
    "crates/unica-coder/Cargo.toml",
    "plugins/unica/.codex-plugin/plugin.json",
    "plugins/unica/.mcp.json",
    "plugins/unica/third-party/tools.lock.json",
    "plugins/unica/third-party/manifest.json",
    "scripts/ci/build-unica-tools.py",
    "scripts/ci/package-unica-plugin.py",
    "scripts/install-unica.sh",
}


def normalize_path(path: str) -> str:
    path = path.strip()
    if path.startswith("./"):
        return path[2:]
    return path


def needs_release_artifacts(paths: Iterable[str]) -> bool:
    return any(normalize_path(path) in RELEASE_ARTIFACT_PATHS for path in paths)


def classify_stdin(stdin: TextIO) -> str:
    return "true" if needs_release_artifacts(stdin) else "false"


def main() -> None:
    print(classify_stdin(sys.stdin))


if __name__ == "__main__":
    main()
