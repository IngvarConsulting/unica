#!/usr/bin/env python3
"""Deterministic rlm-bsl-index console stand-in for reader parity."""

from __future__ import annotations

import os
import sys
from pathlib import Path


def main() -> int:
    configured_root = os.environ.get("RLM_INDEX_DIR")
    if not configured_root:
        print("RLM_INDEX_DIR must be set for reader parity", file=sys.stderr)
        return 2
    index_root = Path(configured_root) / "reader-parity"
    index_root.mkdir(parents=True, exist_ok=True)
    database = index_root / "bsl_index.db"
    action = sys.argv[2] if len(sys.argv) > 2 and sys.argv[1] == "index" else ""
    if action in {"build", "update"}:
        database.touch()
    if action == "info" and not database.is_file():
        print("Index not found")
        return 0
    if database.is_file():
        print(f"Index: {database}")
        print("  Status:   fresh")
        return 0
    print(f"unsupported rlm index invocation: {sys.argv[1:]}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
