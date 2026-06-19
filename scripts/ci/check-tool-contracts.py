#!/usr/bin/env python3
"""Smoke-check bundled Unica tool contracts after product updates."""

from __future__ import annotations

import argparse
import sqlite3
import subprocess
from pathlib import Path


TOOL_HELP_CHECKS = [
    (
        "bsl-analyzer analyze source-dir/jsonl",
        "run-bsl-analyzer.sh",
        ["analyze", "--help"],
        ["--source-dir", "--format", "jsonl"],
    ),
    ("bsl-analyzer search namespace", "run-bsl-analyzer.sh", ["search", "--help"], ["baseline"]),
    (
        "bsl-analyzer mcp workspace stdio",
        "run-bsl-analyzer.sh",
        ["mcp", "serve", "--help"],
        ["--profile", "--source-dir", "--mode", "stdio"],
    ),
    ("bsl-analyzer smoke", "run-bsl-analyzer.sh", ["smoke", "--help"], ["--scenarios", "--json"]),
    ("rlm-bsl-index build", "run-rlm-bsl-index.sh", ["index", "build", "--help"], ["build"]),
    ("rlm-bsl-index update", "run-rlm-bsl-index.sh", ["index", "update", "--help"], ["update"]),
    ("rlm-bsl-index info", "run-rlm-bsl-index.sh", ["index", "info", "--help"], ["info"]),
    ("v8-runner version", "run-v8-runner.sh", ["--version"], ["v8-runner"]),
    ("v8-runner build", "run-v8-runner.sh", ["build", "--help"], ["build"]),
]

RLM_SCHEMA_COLUMNS = {
    "index_meta": {"key", "value"},
    "modules": {"id", "rel_path", "object_name", "category", "module_type"},
    "methods": {"id", "module_id", "name", "type", "is_export", "line", "end_line", "params", "loc"},
    "methods_fts": {"name", "object_name"},
    "regions": {"id", "module_id", "name", "line", "end_line"},
    "module_headers": {"module_id", "header_comment"},
}

RLM_REQUIRED_META = {
    "builder_version": "14",
}


def run_command(command: list[str], cwd: Path) -> tuple[int, str]:
    result = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    return result.returncode, result.stdout + result.stderr


def check_tool_contracts(scripts_dir: Path) -> list[str]:
    scripts_dir = scripts_dir.resolve()
    errors: list[str] = []
    for label, script_name, args, expected_tokens in TOOL_HELP_CHECKS:
        script = scripts_dir / script_name
        if not script.exists():
            errors.append(f"{label}: launcher not found: {script}")
            continue
        status, output = run_command([str(script), *args], scripts_dir)
        if status != 0:
            errors.append(f"{label}: command exited with {status}: {' '.join([script_name, *args])}")
            continue
        for token in expected_tokens:
            if token not in output:
                errors.append(f"{label}: expected token not found in output: {token}")
    return errors


def sqlite_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    rows = conn.execute(f"PRAGMA table_info({table})").fetchall()
    return {row[1] for row in rows}


def check_rlm_schema(db_path: Path) -> list[str]:
    errors: list[str] = []
    if not db_path.exists():
        return [f"RLM index DB not found: {db_path}"]
    with sqlite3.connect(db_path) as conn:
        existing_tables = {
            row[0]
            for row in conn.execute("SELECT name FROM sqlite_master WHERE type IN ('table', 'virtual table')")
        }
        for table, required_columns in RLM_SCHEMA_COLUMNS.items():
            if table not in existing_tables:
                errors.append(f"missing RLM table: {table}")
                continue
            columns = sqlite_columns(conn, table)
            for column in sorted(required_columns - columns):
                errors.append(f"missing RLM column: {table}.{column}")
        if "index_meta" in existing_tables:
            for key, expected in RLM_REQUIRED_META.items():
                row = conn.execute("SELECT value FROM index_meta WHERE key = ?", (key,)).fetchone()
                actual = row[0] if row else None
                if actual != expected:
                    errors.append(f"RLM index_meta {key} must be {expected}, got {actual or '<missing>'}")
    return errors


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scripts-dir", type=Path, default=Path("plugins/unica/scripts"))
    parser.add_argument("--rlm-db", type=Path)
    args = parser.parse_args()

    errors = check_tool_contracts(args.scripts_dir)
    if args.rlm_db:
        errors.extend(check_rlm_schema(args.rlm_db))

    if errors:
        print("Tool contract check failed:")
        for error in errors:
            print(f"- {error}")
        raise SystemExit(1)
    print("Tool contract check passed")


if __name__ == "__main__":
    main()
