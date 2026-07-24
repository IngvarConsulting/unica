#!/usr/bin/env python3
"""Smoke-check bundled Unica tool contracts after product updates."""

from __future__ import annotations

import argparse
import os
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Callable


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


def run_rlm_command(
    command: list[str],
    cwd: Path,
    env: dict[str, str],
    timeout: float = 120.0,
) -> tuple[int, str]:
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env={**os.environ, **env},
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as exc:
        return 1, f"timed out after {timeout}s: {exc}"
    return result.returncode, result.stdout + result.stderr


def check_rlm_mtime_recovery_contract(
    tool: Path,
    *,
    run_rlm: Callable[
        [list[str], Path, dict[str, str]], tuple[int, str]
    ] = run_rlm_command,
) -> list[str]:
    errors: list[str] = []
    with tempfile.TemporaryDirectory(prefix="unica-rlm-mtime-") as tmp:
        root = Path(tmp)
        workspace = root / "workspace"
        modules = [
            workspace / "src" / "CommonModules" / name / "Module.bsl"
            for name in ("ContractOne", "ContractTwo")
        ]
        for number, module in enumerate(modules, start=1):
            module.parent.mkdir(parents=True)
            module.write_text(
                f"Процедура ContractTest{number}() Экспорт\n"
                "    Возврат;\n"
                "КонецПроцедуры\n",
                encoding="utf-8",
            )

        git_without_signing = [
            "git",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgSign=false",
        ]
        git_commands = [
            [*git_without_signing, "init", "-q"],
            [
                *git_without_signing,
                "config",
                "user.email",
                "unica-ci@example.invalid",
            ],
            [*git_without_signing, "config", "user.name", "Unica CI"],
            [*git_without_signing, "add", "."],
            [*git_without_signing, "commit", "-q", "-m", "fixture"],
        ]
        for command in git_commands:
            status, output = run_command(command, workspace)
            if status != 0:
                return [
                    "rlm mtime recovery: failed to prepare clean Git fixture: "
                    f"{' '.join(command)}: {output.strip()}"
                ]

        env = {
            "RLM_INDEX_DIR": str(root / "index"),
            "RLM_INDEX_SAMPLE_SIZE": "1000",
            "RLM_INDEX_SAMPLE_THRESHOLD": "0",
            "RLM_INDEX_SKIP_SAMPLE_HOURS": "0",
        }

        def invoke(action: str) -> str | None:
            command = [str(tool), "index", action, str(workspace)]
            status, output = run_rlm(command, workspace, env)
            if status != 0:
                errors.append(
                    f"rlm mtime recovery: {action} exited with {status}: {output.strip()}"
                )
                return None
            return output

        if invoke("build") is None:
            return errors
        initial_info = invoke("info")
        if initial_info is None:
            return errors
        if "fresh" not in initial_info.lower():
            errors.append("rlm mtime recovery: initial build did not produce fresh info")
            return errors

        for module in modules:
            original = module.stat()
            drifted_mtime_ns = original.st_mtime_ns + 2_000_000_000
            os.utime(module, ns=(original.st_atime_ns, drifted_mtime_ns))
            if module.stat().st_size != original.st_size:
                errors.append("rlm mtime recovery: mtime drift changed fixture size")
                return errors
        git_status, git_output = run_command(
            ["git", "status", "--porcelain", "--untracked-files=no"],
            workspace,
        )
        if git_status != 0 or git_output.strip():
            errors.append(
                "rlm mtime recovery: mtime-only fixture is not Git-clean: "
                f"{git_output.strip()}"
            )
            return errors

        stale_info = invoke("info")
        if stale_info is None:
            return errors
        if "stale (content)" not in stale_info.lower():
            errors.append(
                "rlm mtime recovery: mtime drift did not produce stale (content): "
                f"{stale_info.strip()}"
            )
            return errors

        update = invoke("update")
        if update is None:
            return errors
        if "Changed: 0" not in update or "Fast path: True" not in update:
            errors.append(
                "rlm mtime recovery: update did not report Changed: 0 and Fast path: True"
            )
            return errors

        post_update_info = invoke("info")
        if post_update_info is None:
            return errors
        if "stale (content)" not in post_update_info.lower():
            errors.append(
                "rlm mtime recovery: fast-path update did not remain stale (content)"
            )
            return errors

        if invoke("build") is None:
            return errors
        final_info = invoke("info")
        if final_info is None:
            return errors
        if "fresh" not in final_info.lower():
            errors.append("rlm mtime recovery: full rebuild did not restore fresh info")
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
    rlm_index = tool_executable(tools_dir.resolve(), "rlm-bsl-index", target)
    if rlm_index.exists():
        errors.extend(check_rlm_mtime_recovery_contract(rlm_index))
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
