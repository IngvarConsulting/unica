#!/usr/bin/env python3
"""Validate the release version shared by Unica package contracts."""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path


def read_version_contract(repo_root: Path) -> dict[str, str]:
    workspace = tomllib.loads((repo_root / "Cargo.toml").read_text(encoding="utf-8"))
    plugin = json.loads(
        (repo_root / "plugins" / "unica" / ".codex-plugin" / "plugin.json").read_text(
            encoding="utf-8"
        )
    )
    tools_lock = json.loads(
        (repo_root / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
            encoding="utf-8"
        )
    )
    unica_tools = [tool for tool in tools_lock.get("tools", []) if tool.get("name") == "unica"]
    if len(unica_tools) != 1:
        raise ValueError(f"tools lock must contain exactly one unica entry, found {len(unica_tools)}")

    return {
        "workspace": workspace["workspace"]["package"]["version"],
        "plugin": plugin["version"],
        "tools-lock-unica": unica_tools[0]["version"],
    }


def validate_version_contract(
    values: dict[str, str], *, expected: str | None = None
) -> list[str]:
    if not values:
        return ["version contract is empty"]
    expected_version = expected or next(iter(values.values()))
    return [
        f"{name} version {version} != expected {expected_version}"
        for name, version in values.items()
        if version != expected_version
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    parser.add_argument("--expected")
    args = parser.parse_args()

    try:
        values = read_version_contract(args.repo_root.resolve())
    except (KeyError, OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"version contract error: {error}", file=sys.stderr)
        return 1

    errors = validate_version_contract(values, expected=args.expected)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1

    version = args.expected or next(iter(values.values()))
    print(f"Unica version contract: {version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
