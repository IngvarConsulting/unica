#!/usr/bin/env python3
"""Capture deterministic Codex plugin discovery contracts for compatibility tests."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any


MARKETPLACE_COMMAND = ("plugin", "marketplace", "list", "--json")
PLUGIN_COMMAND = ("plugin", "list", "--available", "--json")
CODEX_VERSION_PATTERN = re.compile(r"^codex-cli [0-9A-Za-z.+-]+$")


def run_codex(codex: Path, codex_home: Path, arguments: tuple[str, ...]) -> str:
    env = os.environ.copy()
    env["CODEX_HOME"] = str(codex_home)
    result = subprocess.run(
        [str(codex), *arguments],
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        detail = sanitize_text(result.stderr.strip(), codex_home)
        raise RuntimeError(
            f"codex {' '.join(arguments)} failed with code {result.returncode}: {detail}"
        )
    return result.stdout.strip()


def sanitize_text(value: str, codex_home: Path) -> str:
    root = str(codex_home)
    variants = {root, root.replace("\\", "/"), root.replace("/", "\\")}
    windows_root = any(re.match(r"^[A-Za-z]:[\\/]", variant) for variant in variants)
    flags = re.IGNORECASE if windows_root else 0
    sanitized = value
    for variant in sorted(variants, key=len, reverse=True):
        sanitized = re.sub(
            re.escape(variant) + r"(?=$|[\\/])",
            "${CODEX_HOME}",
            sanitized,
            flags=flags,
        )
    return sanitized


def sanitize_json(value: Any, codex_home: Path) -> Any:
    if isinstance(value, dict):
        return {key: sanitize_json(item, codex_home) for key, item in value.items()}
    if isinstance(value, list):
        return [sanitize_json(item, codex_home) for item in value]
    if isinstance(value, str):
        return sanitize_text(value, codex_home)
    return value


def reject_external_paths(value: Any) -> None:
    if isinstance(value, dict):
        for item in value.values():
            reject_external_paths(item)
    elif isinstance(value, list):
        for item in value:
            reject_external_paths(item)
    elif isinstance(value, str) and (
        value.startswith(("/", "\\\\")) or re.match(r"^[A-Za-z]:[\\/]", value)
    ):
        raise RuntimeError("captured JSON contains a path outside CODEX_HOME")


def parse_json(label: str, output: str) -> Any:
    try:
        return json.loads(output)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"invalid Codex {label} JSON: {error}") from error


def write_json(path: Path, value: Any) -> None:
    path.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def capture(
    codex: Path,
    codex_home: Path,
    expected_version: str,
    output_dir: Path,
) -> None:
    if not codex.is_file():
        raise RuntimeError(f"Codex executable does not exist: {codex}")
    if not codex_home.is_dir():
        raise RuntimeError(f"isolated CODEX_HOME does not exist: {codex_home}")
    if output_dir.exists():
        raise RuntimeError(f"contract output already exists: {output_dir}")
    if not CODEX_VERSION_PATTERN.fullmatch(expected_version):
        raise RuntimeError("--expected-version must match 'codex-cli <version>'")

    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{output_dir.name}.tmp-", dir=output_dir.parent)
    )
    try:
        version = run_codex(codex, codex_home, ("--version",))
        if version != expected_version:
            raise RuntimeError("Codex version does not match --expected-version")
        marketplaces = parse_json(
            "marketplace",
            run_codex(codex, codex_home, MARKETPLACE_COMMAND),
        )
        plugins = parse_json(
            "plugin",
            run_codex(codex, codex_home, PLUGIN_COMMAND),
        )
        marketplaces = sanitize_json(marketplaces, codex_home)
        plugins = sanitize_json(plugins, codex_home)
        reject_external_paths(marketplaces)
        reject_external_paths(plugins)
        write_json(temporary / "marketplaces.json", marketplaces)
        write_json(temporary / "plugins.json", plugins)
        write_json(
            temporary / "metadata.json",
            {
                "codexVersion": expected_version,
                "commands": [
                    "codex plugin marketplace list --json",
                    "codex plugin list --available --json",
                ],
                "sanitizedRoot": "${CODEX_HOME}",
            },
        )
        os.replace(temporary, output_dir)
    except Exception:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--codex", type=Path, required=True)
    parser.add_argument("--codex-home", type=Path, required=True)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        capture(
            args.codex.resolve(),
            args.codex_home.resolve(),
            args.expected_version,
            args.output_dir.resolve(),
        )
    except RuntimeError as error:
        parser.exit(1, f"capture-codex-contract: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
