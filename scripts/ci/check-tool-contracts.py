#!/usr/bin/env python3
"""Smoke-check bundled Unica tool contracts after product updates."""

from __future__ import annotations

import argparse
import json
import os
import shutil
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

V8_RUNNER_BOUNDED_OUTPUT_MARKER = "bounded-platform-out"
V8_RUNNER_BOUNDED_STDERR_MARKER = "bounded-client-stderr"


def validate_v8_runner_partial_load_list(payload: bytes, expected_path: str) -> list[str]:
    errors: list[str] = []
    bom = b"\xef\xbb\xbf"
    if not payload.startswith(bom):
        errors.append("v8-runner partial-load list is missing UTF-8 BOM")
        text_payload = payload
    else:
        text_payload = payload[len(bom) :]
    if b"\n" in text_payload and b"\r\n" not in text_payload:
        errors.append("v8-runner partial-load list does not use CRLF line endings")
    try:
        contents = text_payload.decode("utf-8")
    except UnicodeDecodeError as error:
        errors.append(f"v8-runner partial-load list is not valid UTF-8: {error}")
        return errors
    if expected_path not in contents:
        errors.append(
            f"v8-runner partial-load list is missing expected Cyrillic path: {expected_path}"
        )
    return errors


def check_v8_runner_partial_load_contract(runner: Path, target: str) -> list[str]:
    label = "v8-runner partial-load contract"
    if not runner.is_file():
        return [f"{label}: binary not found: {runner}"]

    with tempfile.TemporaryDirectory(prefix="unica-v8-runner-179-") as directory:
        root = Path(directory)
        source_root = root / "project" / "main"
        object_root = source_root / "Catalogs.Товары"
        work_path = root / "work"
        infobase_path = root / "ib"
        captured_list = root / "partial-load.lst"
        object_root.mkdir(parents=True)
        work_path.mkdir()
        infobase_path.mkdir()
        (object_root / "ObjectModule.bsl").write_text(
            "Procedure Проверка()\nEndProcedure\n",
            encoding="utf-8",
        )
        (object_root / "ObjectModule.xml").write_text(
            "<MetaDataObject />\n",
            encoding="utf-8",
        )

        stub_source = root / "platform-stub.rs"
        stub_source.write_text(
            """
use std::{env, ffi::OsString, fs, path::PathBuf};

fn main() {
    let mut previous: Option<OsString> = None;
    for argument in env::args_os().skip(1) {
        if previous.as_deref() == Some(std::ffi::OsStr::new("-listFile")) {
            let destination = PathBuf::from(
                env::var_os("UNICA_V8_RUNNER_CAPTURE_LIST")
                    .expect("UNICA_V8_RUNNER_CAPTURE_LIST"),
            );
            fs::copy(&argument, destination).expect("copy partial-load list");
        }
        if previous.as_deref() == Some(std::ffi::OsStr::new("/Out")) {
            fs::write(&argument, b"platform stub completed\\n").expect("write /Out log");
        }
        previous = Some(argument);
    }
}
""".lstrip(),
            encoding="utf-8",
        )
        platform = root / ("1cv8.exe" if target == "win-x64" else "1cv8")
        compiled = subprocess.run(
            ["rustc", "--edition=2021", str(stub_source), "-o", str(platform)],
            cwd=root,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if compiled.returncode != 0:
            return [f"{label}: failed to compile platform stub: {compiled.stderr.strip()}"]

        def yaml_path(path: Path) -> str:
            return str(path).replace("'", "''")

        config = root / "v8project.yaml"
        config.write_text(
            "\n".join(
                [
                    f"workPath: '{yaml_path(work_path)}'",
                    "format: DESIGNER",
                    "builder: DESIGNER",
                    "infobase:",
                    f"  connection: 'File={yaml_path(infobase_path)}'",
                    "build:",
                    "  partialLoadThreshold: 20",
                    "source-set:",
                    "  - name: main",
                    "    type: CONFIGURATION",
                    "    path: project/main",
                    "tools:",
                    "  platform:",
                    f"    path: '{yaml_path(platform)}'",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        environment = os.environ.copy()
        environment["UNICA_V8_RUNNER_CAPTURE_LIST"] = str(captured_list)
        command = [
            str(runner),
            "--config",
            str(config),
            "--json-message",
            "build",
        ]
        initial = subprocess.run(
            command,
            cwd=root,
            env=environment,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if initial.returncode != 0:
            detail = (initial.stderr or initial.stdout).strip()
            return [f"{label}: baseline build exited with {initial.returncode}: {detail}"]
        captured_list.unlink(missing_ok=True)
        (object_root / "ObjectModule.bsl").write_text(
            "Procedure Проверка()\n    // Изменено после baseline\nEndProcedure\n",
            encoding="utf-8",
        )
        result = subprocess.run(
            command,
            cwd=root,
            env=environment,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            return [f"{label}: runner exited with {result.returncode}: {detail}"]
        if not captured_list.is_file():
            return [f"{label}: platform stub did not receive a partial-load list"]
        try:
            envelope = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            return [f"{label}: runner returned invalid JSON: {error}"]
        steps = envelope.get("data", {}).get("steps", [])
        if not steps or "partial" not in json.dumps(steps[0].get("mode", "")).lower():
            return [
                f"{label}: runner did not select partial build mode: "
                f"{json.dumps(steps, ensure_ascii=False)}"
            ]

        expected_path = str(Path("Catalogs.Товары") / "ObjectModule.bsl")
        return [
            f"{label}: {error}"
            for error in validate_v8_runner_partial_load_list(
                captured_list.read_bytes(),
                expected_path,
            )
        ]


def validate_v8_runner_bounded_external_epf_result(
    envelope: object,
    execute: Path,
    output: Path,
    stderr_output: Path,
    output_marker: str,
    stderr_marker: str,
) -> list[str]:
    errors: list[str] = []
    data = envelope.get("data") if isinstance(envelope, dict) else None
    wait = data.get("external_epf_wait") if isinstance(data, dict) else None
    if not isinstance(wait, dict):
        return ["runner JSON is missing data.external_epf_wait"]

    pid = wait.get("pid")
    if isinstance(pid, bool) or not isinstance(pid, int) or pid <= 0:
        errors.append(f"external_epf_wait.pid must be a positive integer, got {pid!r}")
    if wait.get("exit_code") != 7:
        errors.append(
            f"external_epf_wait.exit_code must be 7, got {wait.get('exit_code')!r}"
        )
    if wait.get("timed_out") is not False:
        errors.append(
            f"external_epf_wait.timed_out must be false, got {wait.get('timed_out')!r}"
        )

    expected_paths = {
        "execute_path": execute,
        "output_path": output,
        "stderr_path": stderr_output,
    }
    for field, expected in expected_paths.items():
        actual = wait.get(field)
        if not isinstance(actual, str):
            errors.append(f"external_epf_wait.{field} must be a path string, got {actual!r}")
            continue
        if Path(actual).resolve() != expected.resolve():
            errors.append(
                f"external_epf_wait.{field} must be {expected.resolve()}, got {actual}"
            )

    if not output.is_file():
        errors.append(f"platform /Out artifact was not created: {output}")
    else:
        try:
            output_contents = output.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            errors.append(f"platform /Out artifact could not be read: {error}")
        else:
            if output_marker not in output_contents:
                errors.append(
                    "platform /Out artifact does not contain expected marker: "
                    f"{output_marker}"
                )
            if stderr_marker in output_contents:
                errors.append(
                    "platform /Out artifact unexpectedly contains stderr marker: "
                    f"{stderr_marker}"
                )
    if not stderr_output.is_file():
        errors.append(f"stderr artifact was not created: {stderr_output}")
    else:
        try:
            stderr_contents = stderr_output.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            errors.append(f"stderr artifact could not be read: {error}")
        else:
            if stderr_marker not in stderr_contents:
                errors.append(
                    f"stderr artifact does not contain expected marker: {stderr_marker}"
                )
            if output_marker in stderr_contents:
                errors.append(
                    "stderr artifact unexpectedly contains platform /Out marker: "
                    f"{output_marker}"
                )
    return errors


def check_v8_runner_bounded_external_epf_contract(
    runner: Path,
    target: str,
) -> list[str]:
    label = "v8-runner bounded external EPF contract"
    if not runner.is_file():
        return [f"{label}: binary not found: {runner}"]

    with tempfile.TemporaryDirectory(prefix="unica-v8-runner-110-") as directory:
        root = Path(directory)
        project_root = root / "project"
        work_path = root / "work"
        infobase_path = root / "ib"
        platform_root = root / "platform"
        platform_bin = platform_root / "bin"
        execute = root / "processor.epf"
        output = root / "platform.log"
        stderr_output = root / "client.stderr.log"
        project_root.mkdir()
        work_path.mkdir()
        infobase_path.mkdir()
        platform_bin.mkdir(parents=True)
        execute.write_bytes(b"bounded external EPF contract\n")

        stub_source = root / "platform-stub.rs"
        stub_source.write_text(
            f"""
use std::{{env, fs, process}};

fn main() {{
    let executable = env::current_exe().expect("resolve platform stub");
    let name = executable
        .file_stem()
        .expect("platform stub name")
        .to_string_lossy();
    if !name.eq_ignore_ascii_case("1cv8c") {{
        return;
    }}

    let arguments: Vec<_> = env::args_os().skip(1).collect();
    for pair in arguments.windows(2) {{
        if pair[0].to_string_lossy().eq_ignore_ascii_case("/Out") {{
            fs::write(&pair[1], b"{V8_RUNNER_BOUNDED_OUTPUT_MARKER}\\n")
                .expect("write /Out log");
        }}
    }}
    eprintln!("{V8_RUNNER_BOUNDED_STDERR_MARKER}");
    process::exit(7);
}}
""".lstrip(),
            encoding="utf-8",
        )
        suffix = ".exe" if target == "win-x64" else ""
        client_platform = platform_bin / f"1cv8c{suffix}"
        gui_platform = platform_bin / f"1cv8{suffix}"
        compiled = subprocess.run(
            ["rustc", "--edition=2021", str(stub_source), "-o", str(client_platform)],
            cwd=root,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if compiled.returncode != 0:
            return [f"{label}: failed to compile platform stub: {compiled.stderr.strip()}"]
        shutil.copy2(client_platform, gui_platform)

        def yaml_path(path: Path) -> str:
            return str(path).replace("'", "''")

        config = root / "v8project.yaml"
        config.write_text(
            "\n".join(
                [
                    f"workPath: '{yaml_path(work_path)}'",
                    "format: DESIGNER",
                    "builder: DESIGNER",
                    "infobase:",
                    f"  connection: 'File={yaml_path(infobase_path)}'",
                    "source-set:",
                    "  - name: main",
                    "    type: CONFIGURATION",
                    "    path: project",
                    "tools:",
                    "  platform:",
                    f"    path: '{yaml_path(platform_root)}'",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        command = [
            str(runner),
            "--config",
            str(config),
            "--json-message",
            "launch",
            "thin",
            "--execute",
            str(execute),
            "--output",
            str(output),
            "--stderr-output",
            str(stderr_output),
            "--wait-for-exit",
            "--wait-timeout-ms",
            "30000",
        ]
        try:
            result = subprocess.run(
                command,
                cwd=root,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=60,
                check=False,
            )
        except subprocess.TimeoutExpired:
            return [f"{label}: runner did not exit within 60 seconds"]
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            return [
                f"{label}: runner OS process exited with {result.returncode}; "
                f"expected 0 for external EPF exit 7: {detail}"
            ]
        try:
            envelope = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            return [f"{label}: runner returned invalid JSON: {error}"]
        return [
            f"{label}: {error}"
            for error in validate_v8_runner_bounded_external_epf_result(
                envelope,
                execute,
                output,
                stderr_output,
                V8_RUNNER_BOUNDED_OUTPUT_MARKER,
                V8_RUNNER_BOUNDED_STDERR_MARKER,
            )
        ]


def run_command(
    command: list[str],
    cwd: Path,
    *,
    env: dict[str, str] | None = None,
    timeout: float | None = None,
) -> tuple[int, str]:
    suffix = Path(command[0]).suffix.lower()
    if suffix == ".py":
        command = [sys.executable, *command]
    elif os.name == "nt" and suffix in {".bat", ".cmd"}:
        command = [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/s", "/c", *command]
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=None if env is None else {**os.environ, **env},
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as exc:
        return 1, f"timed out after {timeout}s: {exc}"
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
    if target is not None:
        errors.extend(
            check_v8_runner_partial_load_contract(
                tool_executable(tools_dir, "v8-runner", target),
                target,
            )
        )
        errors.extend(
            check_v8_runner_bounded_external_epf_contract(
                tool_executable(tools_dir, "v8-runner", target),
                target,
            )
        )
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
    return run_command(command, cwd, env=env, timeout=timeout)


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

        head_status, head_before_update = run_command(
            ["git", "rev-parse", "HEAD"],
            workspace,
        )
        if head_status != 0 or not head_before_update.strip():
            errors.append(
                "rlm mtime recovery: failed to read Git HEAD before update: "
                f"{head_before_update.strip()}"
            )
            return errors
        update = invoke("update")
        if update is None:
            return errors
        head_status, head_after_update = run_command(
            ["git", "rev-parse", "HEAD"],
            workspace,
        )
        if head_status != 0 or not head_after_update.strip():
            errors.append(
                "rlm mtime recovery: failed to read Git HEAD after update: "
                f"{head_after_update.strip()}"
            )
            return errors
        if head_after_update.strip() != head_before_update.strip():
            errors.append(
                "rlm mtime recovery: Git HEAD changed during update: "
                f"{head_before_update.strip()} -> {head_after_update.strip()}"
            )
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
