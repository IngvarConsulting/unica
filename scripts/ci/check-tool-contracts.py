#!/usr/bin/env python3
"""Smoke-check bundled Unica tool contracts after product updates."""

from __future__ import annotations

import argparse
import os
import sqlite3
import subprocess
import sys
from pathlib import Path


TOOL_HELP_CHECKS = [
    (
        "bsl-analyzer analyze source-dir/jsonl",
        "bsl-analyzer",
        ["analyze", "--help"],
        ["--source-dir", "--format", "jsonl"],
    ),
    (
        "bsl-analyzer mcp workspace stdio",
        "bsl-analyzer",
        ["mcp", "serve", "--help"],
        ["--profile", "--source-dir", "--mode", "stdio"],
    ),
    ("rlm-bsl-index build", "rlm-bsl-index", ["index", "build", "--help"], ["build"]),
    ("rlm-bsl-index update", "rlm-bsl-index", ["index", "update", "--help"], ["update"]),
    ("rlm-bsl-index info", "rlm-bsl-index", ["index", "info", "--help"], ["info"]),
    (
        "rlm-tools-bsl server",
        "rlm-tools-bsl",
        ["--help"],
        ["--transport", "stdio", "streamable-http"],
    ),
    ("v8-runner version", "v8-runner", ["--version"], ["v8-runner"]),
    ("v8-runner build", "v8-runner", ["build", "--help"], ["build"]),
]

RLM_SCHEMA_COLUMNS = {
    "index_meta": {"key", "value"},
    "modules": {"id", "rel_path", "object_name", "category", "module_type"},
    "methods": {"id", "module_id", "name", "type", "is_export", "line", "end_line", "params", "loc"},
    "methods_fts": {"name", "object_name"},
    "regions": {"id", "module_id", "name", "line", "end_line"},
    "module_headers": {"module_id", "header_comment"},
    "object_attributes": {
        "id",
        "object_name",
        "category",
        "attr_name",
        "attr_type",
        "attr_kind",
        "ts_name",
    },
    "role_rights": {"id", "role_name", "object_name", "right_name", "file"},
    "event_subscriptions": {
        "id",
        "name",
        "event",
        "handler_module",
        "handler_procedure",
        "source_types",
    },
    "functional_options": {"id", "name", "location", "content", "file"},
    "predefined_items": {"id", "object_name", "category", "item_name", "item_code"},
}

RLM_REQUIRED_META = {
    "builder_version": "14",
}


def run_command(command: list[str], cwd: Path) -> tuple[int, str]:
    suffix = Path(command[0]).suffix.lower()
    if suffix == ".py":
        command = [sys.executable, *command]
    elif os.name == "nt" and suffix in {".bat", ".cmd"}:
        command = [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/s", "/c", *command]
    result = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    return result.returncode, result.stdout + result.stderr


def detect_target() -> str:
    import platform

    system = platform.system()
    machine = platform.machine().lower()
    if system == "Darwin" and machine in {"arm64", "aarch64"}:
        return "darwin-arm64"
    if system == "Linux" and machine in {"x86_64", "amd64"}:
        return "linux-x64"
    if system == "Windows" and machine in {"x86_64", "amd64"}:
        return "win-x64"
    raise SystemExit(f"unsupported Unica tool target: {system}-{machine}")


def tool_executable(tools_dir: Path, tool_name: str, target: str | None) -> Path:
    suffix = ".exe" if target == "win-x64" else ""
    candidate = tools_dir / f"{tool_name}{suffix}"
    if candidate.exists() or suffix:
        return candidate
    exe_candidate = tools_dir / f"{tool_name}.exe"
    if exe_candidate.exists():
        return exe_candidate
    if target is None:
        for script_suffix in (".py", ".cmd", ".bat"):
            script_candidate = tools_dir / f"{tool_name}{script_suffix}"
            if script_candidate.exists():
                return script_candidate
    return candidate


def check_tool_contracts(tools_dir: Path, target: str | None = None) -> list[str]:
    tools_dir = tools_dir.resolve()
    errors: list[str] = []
    for label, tool_name, args, expected_tokens in TOOL_HELP_CHECKS:
        tool = tool_executable(tools_dir, tool_name, target)
        if not tool.exists():
            errors.append(f"{label}: binary not found: {tool}")
            continue
        status, output = run_command([str(tool), *args], tools_dir)
        if status != 0:
            errors.append(f"{label}: command exited with {status}: {' '.join([tool.name, *args])}")
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
    conn = sqlite3.connect(db_path)
    try:
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
    finally:
        conn.close()
    return errors


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", default=None)
    parser.add_argument("--tools-dir", type=Path)
    parser.add_argument("--rlm-db", type=Path)
    args = parser.parse_args()

    target = args.target or detect_target()
    tools_dir = args.tools_dir or Path("plugins/unica/bin") / target
    errors = check_tool_contracts(tools_dir, target)
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
