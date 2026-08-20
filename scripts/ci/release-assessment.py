#!/usr/bin/env python3
"""Run Unica release assessment against a pinned 1c-syntax/ssl_3_2 ref."""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
import platform
import queue
import shutil
import subprocess
import sys
import tarfile
import threading
import time
import zipfile
from collections.abc import Callable
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


SCHEMA_VERSION = 1
BSP_REPO = "https://github.com/1c-syntax/ssl_3_2"
BSP_REF = "3.2.1.446"
SOURCE_DIR = "src/cf"
EXPECTED_PUBLIC_TOOLS = {
    "unica.project.status",
    "unica.project.map",
    "unica.cf.info",
    "unica.cf.validate",
    "unica.code.diagnostics",
    "unica.code.search",
    "unica.code.outline",
    "unica.meta.info",
    "unica.meta.add",
    "unica.meta.edit",
    "unica.meta.remove",
    "unica.standards.explain",
}
INDEX_WAIT_TIMEOUT_SECONDS = 300
INDEX_POLL_INTERVAL_SECONDS = 1
INDEXED_SEARCH_ROLES = ("semantic", "symbol")


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def json_digest(value: Any) -> str:
    payload = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def read_json(path: Path, default: dict[str, Any] | None = None) -> dict[str, Any]:
    if not path.is_file():
        return default or {}
    return json.loads(path.read_text(encoding="utf-8-sig"))


def release_tag_from_env() -> str:
    ref = os.environ.get("GITHUB_REF_NAME") or os.environ.get("GITHUB_REF", "")
    if ref.startswith("refs/tags/"):
        return ref.removeprefix("refs/tags/")
    return ref or "manual"


def run_command(command: list[str], cwd: Path) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit(
            f"command failed with {result.returncode}: {' '.join(command)}\n{result.stdout}{result.stderr}"
        )
    return result.stdout.strip()


def download_bsp(work_dir: Path, *, repo: str = BSP_REPO, ref: str = BSP_REF) -> tuple[Path, str]:
    target = work_dir / "ssl_3_2"
    if target.exists():
        shutil.rmtree(target)
    work_dir.mkdir(parents=True, exist_ok=True)
    run_command(["git", "clone", "--depth", "1", "--branch", ref, repo, str(target)], work_dir)
    commit = run_command(["git", "rev-parse", "HEAD"], target)
    return target, commit


def safe_extract_tar(archive: Path, extract_dir: Path) -> None:
    root = extract_dir.resolve()
    with tarfile.open(archive) as tf:
        for member in tf.getmembers():
            target = (extract_dir / member.name).resolve()
            if not str(target).startswith(str(root) + os.sep) and target != root:
                raise SystemExit(f"refusing to extract path outside target directory: {member.name}")
        try:
            tf.extractall(extract_dir, filter="data")
        except TypeError:
            tf.extractall(extract_dir)


def safe_extract_zip(archive: Path, extract_dir: Path) -> None:
    root = extract_dir.resolve()
    with zipfile.ZipFile(archive) as zf:
        for name in zf.namelist():
            target = (extract_dir / name).resolve()
            if not str(target).startswith(str(root) + os.sep) and target != root:
                raise SystemExit(f"refusing to extract path outside target directory: {name}")
        zf.extractall(extract_dir)


def extract_marketplace_archive(archive: Path, extract_dir: Path) -> Path:
    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    extract_dir.mkdir(parents=True)

    if tarfile.is_tarfile(archive):
        safe_extract_tar(archive, extract_dir)
    elif zipfile.is_zipfile(archive):
        safe_extract_zip(archive, extract_dir)
    else:
        raise SystemExit(f"unsupported marketplace archive: {archive}")

    candidates = sorted(
        path
        for pattern in (
            "plugins/unica/bin/*/unica",
            "plugins/unica/bin/*/unica.exe",
            "bin/*/unica",
            "bin/*/unica.exe",
        )
        for path in extract_dir.rglob(pattern)
    )
    if not candidates:
        raise SystemExit(f"bundled unica binary not found after extracting {archive}")
    run_unica = candidates[0]
    run_unica.chmod(run_unica.stat().st_mode | 0o111)
    return run_unica


def plugin_root_for(run_unica: Path) -> Path:
    for parent in run_unica.parents:
        if (parent / ".codex-plugin" / "plugin.json").is_file() or (
            parent / "third-party" / "manifest.json"
        ).is_file():
            return parent
    return run_unica.parent


def overlay_runtime_files(run_unica: Path, overlay_root: Path) -> list[str]:
    """Add explicitly staged engines to a thin runtime assessment.

    The candidate archive remains the byte-identical thin core. The overlay is
    test input for scenarios that intentionally exercise an engine; it may add
    regular files but may not replace anything from the candidate.
    """
    if not overlay_root.is_dir():
        raise SystemExit(f"runtime assessment engine overlay is missing: {overlay_root}")
    runtime_root = plugin_root_for(run_unica)
    copied: list[str] = []
    for source in sorted(overlay_root.rglob("*")):
        if source.is_symlink():
            raise SystemExit(f"runtime assessment overlay contains a symlink: {source}")
        if not source.is_file():
            continue
        relative = source.relative_to(overlay_root)
        destination = runtime_root / relative
        if destination.exists():
            raise SystemExit(
                f"runtime assessment overlay would replace candidate file: {relative.as_posix()}"
            )
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        copied.append(relative.as_posix())
    if not copied:
        raise SystemExit(f"runtime assessment engine overlay is empty: {overlay_root}")
    return copied


def prepare_runtime_overlay(overlay_input: Path, extract_dir: Path) -> Path:
    """Restore an engine overlay without trusting artifact file modes.

    GitHub's zipped artifact transport normalizes ordinary files to 0644. The
    engine layer therefore crosses the job boundary as a tar archive whose
    member modes are part of the verified input.
    """
    if overlay_input.is_dir():
        return overlay_input
    if not overlay_input.is_file() or not tarfile.is_tarfile(overlay_input):
        raise SystemExit(f"unsupported runtime assessment engine overlay: {overlay_input}")
    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    extract_dir.mkdir(parents=True)
    safe_extract_tar(overlay_input, extract_dir)
    return extract_dir


def unica_version(run_unica: Path) -> str:
    plugin_json = read_json(plugin_root_for(run_unica) / ".codex-plugin" / "plugin.json")
    return str(plugin_json.get("version", "unknown"))


def call_mcp(
    run_unica: Path,
    messages: list[dict[str, Any]],
    *,
    cwd: Path,
    cache_dir: Path,
    timeout_seconds: float,
) -> tuple[list[dict[str, Any]], int, str, str, int]:
    cache_dir.mkdir(parents=True, exist_ok=True)
    # The rmcp-based server requires the MCP handshake before requests; prepend
    # it unless the scenario drives initialize itself, and strip its response.
    if not messages or messages[0].get("method") != "initialize":
        messages = [
            {
                "jsonrpc": "2.0",
                "id": MCP_HANDSHAKE_ID,
                "method": "initialize",
                "params": MCP_INITIALIZE_PARAMS,
            },
            {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        ] + messages
    payload = "\n".join(json.dumps(message, ensure_ascii=False) for message in messages) + "\n"
    payload.encode("utf-8", errors="strict")
    env = os.environ.copy()
    env["UNICA_CACHE_DIR"] = str(cache_dir)
    started = time.perf_counter()
    command = [sys.executable, str(run_unica)] if run_unica.suffix == ".py" else [str(run_unica)]
    deadline = time.monotonic() + timeout_seconds
    expected_ids = [message["id"] for message in messages if "id" in message]

    def rpc_id_key(value: Any) -> str:
        return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))

    expected_keys = [rpc_id_key(message_id) for message_id in expected_ids]
    if len(set(expected_keys)) != len(expected_keys):
        raise ValueError("MCP request IDs must be unique")
    expected_key_set = set(expected_keys)

    stdout_queue: queue.Queue[str | None] = queue.Queue()
    stdout_reader_errors: list[str] = []
    stderr_parts: list[str] = []
    responses_by_key: dict[str, dict[str, Any]] = {}
    protocol_errors: list[dict[str, Any]] = []
    stdout_lines: list[str] = []
    process: subprocess.Popen[str] | None = None
    stdout_thread: threading.Thread | None = None
    stderr_thread: threading.Thread | None = None
    timed_out = False
    write_error = ""

    def protocol_error(message: str) -> None:
        protocol_errors.append(
            {"jsonrpc": "2.0", "error": {"code": -32603, "message": message}}
        )

    def consume_stdout_line(line: str) -> None:
        stdout_lines.append(line)
        if not line.strip():
            return
        try:
            response = json.loads(line)
        except json.JSONDecodeError:
            protocol_error(f"invalid JSON-RPC line: {line.rstrip()}")
            return
        if not isinstance(response, dict):
            protocol_error("JSON-RPC response must be an object")
            return
        if "id" not in response:
            if "method" in response:
                return
            protocol_error("JSON-RPC response is missing id")
            return
        key = rpc_id_key(response["id"])
        if key not in expected_key_set:
            protocol_error(f"unexpected JSON-RPC response id: {response['id']!r}")
            return
        if key in responses_by_key:
            protocol_error(f"duplicate JSON-RPC response id: {response['id']!r}")
            return
        responses_by_key[key] = response

    def read_stdout() -> None:
        assert process is not None
        assert process.stdout is not None
        try:
            for line in process.stdout:
                stdout_queue.put(line)
        except (OSError, UnicodeError) as error:
            stdout_reader_errors.append(f"failed to read MCP stdout: {error}")
        finally:
            stdout_queue.put(None)

    def read_stderr() -> None:
        assert process is not None
        assert process.stderr is not None
        try:
            stderr_parts.append(process.stderr.read())
        except (OSError, UnicodeError) as error:
            stderr_parts.append(f"failed to read MCP stderr: {error}")

    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="strict",
            cwd=cwd,
            env=env,
            bufsize=1,
        )
        stdout_thread = threading.Thread(target=read_stdout, daemon=True)
        stderr_thread = threading.Thread(target=read_stderr, daemon=True)
        stdout_thread.start()
        stderr_thread.start()

        assert process.stdin is not None
        try:
            process.stdin.write(payload)
            process.stdin.flush()
        except (BrokenPipeError, OSError, UnicodeError) as error:
            write_error = f"failed to write MCP request: {error}"

        while len(responses_by_key) < len(expected_keys) and not protocol_errors:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                timed_out = True
                break
            try:
                line = stdout_queue.get(timeout=min(remaining, 0.05))
            except queue.Empty:
                if process.poll() is not None and stdout_thread is not None and not stdout_thread.is_alive():
                    break
                continue
            if line is None:
                break
            consume_stdout_line(line)
    finally:
        if process is not None:
            if process.stdin is not None and not process.stdin.closed:
                try:
                    process.stdin.close()
                except (BrokenPipeError, OSError, UnicodeError) as error:
                    if not write_error:
                        write_error = f"failed to close MCP stdin: {error}"

            if process.poll() is None:
                remaining = max(0.0, deadline - time.monotonic())
                try:
                    process.wait(timeout=remaining)
                except subprocess.TimeoutExpired:
                    timed_out = True
                    process.kill()
                    process.wait()

            if stdout_thread is not None:
                stdout_thread.join(timeout=1)
            if stderr_thread is not None:
                stderr_thread.join(timeout=1)
            while True:
                try:
                    line = stdout_queue.get_nowait()
                except queue.Empty:
                    break
                if line is not None:
                    consume_stdout_line(line)
            if process.stdout is not None:
                process.stdout.close()
            if process.stderr is not None:
                process.stderr.close()

    duration_ms = int((time.perf_counter() - started) * 1000)
    stdout = "".join(stdout_lines)
    stderr = "".join(stderr_parts)
    if stdout_reader_errors:
        protocol_error("; ".join(stdout_reader_errors))
    if write_error:
        stderr = f"{write_error}\n{stderr}".strip()
    if timed_out:
        return [], duration_ms, stdout, stderr or f"timed out after {timeout_seconds}s", 124
    handshake_key = rpc_id_key(MCP_HANDSHAKE_ID)
    handshake_response = responses_by_key.get(handshake_key)
    if handshake_response is not None and "error" in handshake_response:
        error = handshake_response["error"]
        protocol_error(f"MCP handshake failed: {error.get('message', error)}")
    responses = [
        responses_by_key[key]
        for key in expected_keys
        if key in responses_by_key and key != handshake_key
    ]
    responses.extend(protocol_errors)
    if len(responses_by_key) < len(expected_keys) and not protocol_errors:
        missing = [
            message_id
            for message_id, key in zip(expected_ids, expected_keys, strict=True)
            if key not in responses_by_key
        ]
        protocol_error(f"missing JSON-RPC responses for ids: {missing!r}")
        responses.extend(protocol_errors)
    returncode = process.returncode if process is not None and process.returncode is not None else 1
    if write_error and returncode == 0:
        returncode = 1
    return responses, duration_ms, stdout, stderr, returncode


MCP_HANDSHAKE_ID = "unica-assessment-handshake"
MCP_INITIALIZE_PARAMS = {
    "protocolVersion": "2025-06-18",
    "capabilities": {},
    "clientInfo": {"name": "unica-release-assessment", "version": "1"},
}


def tool_call_message(
    message_id: int,
    name: str,
    arguments: dict[str, Any],
    *,
    progress_token: str | None = None,
) -> dict[str, Any]:
    params: dict[str, Any] = {"name": name, "arguments": arguments}
    if progress_token is not None:
        params["_meta"] = {"progressToken": progress_token}
    return {
        "jsonrpc": "2.0",
        "id": message_id,
        "method": "tools/call",
        "params": params,
    }


def search_progress_snapshots(stdout: str, progress_token: str) -> list[dict[str, Any]]:
    snapshots: list[dict[str, Any]] = []
    for line in stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(message, dict) or message.get("method") != "notifications/progress":
            continue
        params = message.get("params")
        if not isinstance(params, dict) or params.get("progressToken") != progress_token:
            continue
        meta = params.get("_meta")
        snapshot = meta.get("io.unica/searchProgress") if isinstance(meta, dict) else None
        if isinstance(snapshot, dict):
            snapshots.append(snapshot)
    return snapshots


def parse_tool_payload(response: dict[str, Any]) -> tuple[dict[str, Any] | None, list[str]]:
    if "error" in response:
        error = response["error"]
        return None, [str(error.get("message", error))]
    try:
        text = response["result"]["content"][0]["text"]
    except (KeyError, IndexError, TypeError):
        return None, ["tool response does not contain text content"]
    try:
        payload = json.loads(text)
    except json.JSONDecodeError:
        payload = {"ok": True, "stdout": text, "warnings": [], "errors": [], "artifacts": []}
    if not isinstance(payload, dict):
        return None, ["tool text payload is not a JSON object"]
    return payload, []


def response_output_size(stdout: str, stderr: str, payload: dict[str, Any] | None) -> int:
    payload_size = 0 if payload is None else len(json.dumps(payload, ensure_ascii=False).encode("utf-8"))
    return len(stdout.encode("utf-8")) + len(stderr.encode("utf-8")) + payload_size


def project_source_sets(payload: dict[str, Any] | None) -> list[dict[str, Any]]:
    """Read the source sets from the typed result.

    ADR-0023 moved the map out of `stdout`, where it used to be a JSON string
    inside the JSON envelope; `data` is the only place it lives now.
    """

    if not payload:
        return []
    data = payload.get("data")
    for candidate in (data if isinstance(data, dict) else {}, payload):
        source_sets = candidate.get("sourceSets")
        if source_sets is None:
            source_sets = candidate.get("source_sets")
        if isinstance(source_sets, list):
            return [item for item in source_sets if isinstance(item, dict)]
    return []


def project_source_set_name(payload: dict[str, Any] | None, source_path: str) -> str | None:
    for source_set in project_source_sets(payload):
        if source_set.get("path") == source_path:
            name = source_set.get("name")
            if isinstance(name, str) and name.strip() == name and name:
                return name
    return None


def scenario_result(
    *,
    scenario_id: str,
    title: str,
    tool: str,
    arguments: Any,
    status: str,
    duration_ms: int,
    blocking: bool,
    metrics: dict[str, Any] | None = None,
    errors: list[str] | None = None,
    artifacts: list[str] | None = None,
) -> dict[str, Any]:
    return {
        "id": scenario_id,
        "title": title,
        "tool": tool,
        "argumentsDigest": json_digest(arguments),
        "status": status,
        "durationMs": duration_ms,
        "blocking": blocking,
        "metrics": metrics or {},
        "errors": errors or [],
        "artifacts": artifacts or [],
    }


def run_tools_list_scenario(run_unica: Path, bsp_root: Path, cache_dir: Path, timeout_seconds: int) -> dict[str, Any]:
    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": MCP_INITIALIZE_PARAMS},
        {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
    ]
    responses, duration_ms, stdout, stderr, returncode = call_mcp(
        run_unica,
        messages,
        cwd=bsp_root,
        cache_dir=cache_dir,
        timeout_seconds=timeout_seconds,
    )
    errors: list[str] = []
    tools: set[str] = set()
    if returncode != 0:
        errors.append(f"unica exited with {returncode}: {stderr.strip()}")
    if len(responses) != 2:
        errors.append(f"expected 2 JSON-RPC responses, got {len(responses)}")
    else:
        server_name = responses[0].get("result", {}).get("serverInfo", {}).get("name")
        if server_name != "unica":
            errors.append(f"expected serverInfo.name=unica, got {server_name!r}")
        tools = {tool.get("name", "") for tool in responses[1].get("result", {}).get("tools", [])}
        missing = sorted(EXPECTED_PUBLIC_TOOLS - tools)
        if missing:
            errors.append(f"missing expected public tools: {', '.join(missing)}")
    metrics = {
        "toolsCount": len(tools),
        "outputBytes": response_output_size(stdout, stderr, None),
    }
    return scenario_result(
        scenario_id="mcp-tools-list",
        title="MCP initialize and public tools list",
        tool="initialize+tools/list",
        arguments=messages,
        status="failed" if errors else "passed",
        duration_ms=duration_ms,
        blocking=True,
        metrics=metrics,
        errors=errors,
    )


def run_tool_scenario(
    run_unica: Path,
    *,
    bsp_root: Path,
    cache_dir: Path,
    scenario_id: str,
    title: str,
    tool: str,
    arguments: dict[str, Any],
    timeout_seconds: float,
    blocking: bool,
    require_payload_ok: bool,
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    args = dict(arguments)
    args.setdefault("cwd", str(bsp_root))
    progress_token = "release-assessment-code-search" if tool == "unica.code.search" else None
    message = tool_call_message(1, tool, args, progress_token=progress_token)
    responses, duration_ms, stdout, stderr, returncode = call_mcp(
        run_unica,
        [message],
        cwd=bsp_root,
        cache_dir=cache_dir,
        timeout_seconds=timeout_seconds,
    )
    errors: list[str] = []
    payload: dict[str, Any] | None = None
    if returncode != 0:
        errors.append(f"unica exited with {returncode}: {stderr.strip()}")
    if len(responses) != 1:
        errors.append(f"expected 1 JSON-RPC response, got {len(responses)}")
    elif not errors:
        payload, payload_errors = parse_tool_payload(responses[0])
        errors.extend(payload_errors)

    if payload is not None:
        payload_errors = [str(item) for item in payload.get("errors", []) if str(item).strip()]
        if payload.get("ok") is False and require_payload_ok:
            errors.extend(payload_errors or [str(payload.get("summary", f"{tool} reported ok=false"))])

    progress_snapshots = (
        search_progress_snapshots(stdout, progress_token)
        if progress_token is not None
        else []
    )
    if progress_token is not None:
        if not progress_snapshots:
            errors.append("code search did not publish typed MCP progress")
        else:
            terminal = progress_snapshots[-1].get("providers")
            roles = (
                [provider.get("role") for provider in terminal if isinstance(provider, dict)]
                if isinstance(terminal, list)
                else []
            )
            states = (
                [provider.get("state") for provider in terminal if isinstance(provider, dict)]
                if isinstance(terminal, list)
                else []
            )
            if roles != ["semantic", "symbol", "lexical"] or any(
                state not in {"completed", "unavailable", "failed", "timedOut", "cancelled"}
                for state in states
            ):
                errors.append("code search progress did not reach all three terminal roles")

    metrics = {
        "outputBytes": response_output_size(stdout, stderr, payload),
        "warningsCount": len(payload.get("warnings", [])) if payload else 0,
        "errorsCount": len(payload.get("errors", [])) if payload else len(errors),
    }
    if progress_token is not None:
        metrics["progressNotifications"] = len(progress_snapshots)
    source_sets = project_source_sets(payload)
    if source_sets:
        metrics["sourceSetsCount"] = len(source_sets)
    if payload and "cache" in payload:
        metrics["cache"] = payload["cache"]
    status = "failed" if errors else "passed"
    result = scenario_result(
        scenario_id=scenario_id,
        title=title,
        tool=tool,
        arguments=args,
        status=status,
        duration_ms=duration_ms,
        blocking=blocking,
        metrics=metrics,
        errors=errors,
        artifacts=[str(item) for item in payload.get("artifacts", [])] if payload else [],
    )
    return result, payload


def validate_project_map(scenario: dict[str, Any], payload: dict[str, Any] | None) -> None:
    if payload is None:
        return
    source_sets = project_source_sets(payload)
    if not source_sets:
        scenario["status"] = "failed"
        scenario["errors"].append("project map payload does not contain sourceSets")
        return
    found = any(
        item.get("path") == SOURCE_DIR and item.get("sourceFormat") in {"platform_xml", "PlatformXml"}
        for item in source_sets
        if isinstance(item, dict)
    )
    if not found:
        scenario["status"] = "failed"
        scenario["errors"].append(f"project map did not detect {SOURCE_DIR} as platform XML")


def is_non_negative_integer(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def valid_code_search_match_count(section: dict[str, Any]) -> bool:
    matches = section.get("matches")
    hits = section.get("hits")
    if not isinstance(matches, dict) or not isinstance(hits, list):
        return False
    returned = matches.get("returned")
    relation = matches.get("relation")
    if (
        not is_non_negative_integer(returned)
        or returned != len(hits)
        or relation not in {"exact", "lowerBound", "unknown"}
    ):
        return False
    total_present = "total" in matches
    total = matches.get("total")
    if total_present and not is_non_negative_integer(total):
        return False
    if relation == "exact" and total != returned:
        return False
    if relation == "lowerBound" and (not total_present or total < returned):
        return False
    if relation == "unknown" and total_present:
        return False

    status = section.get("status")
    search_complete = section.get("searchComplete")
    if status == "ok":
        return search_complete is True and returned > 0 and relation == "exact"
    if status == "empty":
        return search_complete is True and returned == 0 and relation == "exact"
    if status in {"limitReached", "timedOut"}:
        return search_complete is False and relation == "lowerBound"
    if status in {"unavailable", "failed"}:
        return search_complete is False and returned == 0 and relation == "unknown"
    return False


def valid_code_search_termination(section: dict[str, Any]) -> bool:
    if "termination" not in section:
        return False
    status = section.get("status")
    termination = section["termination"]
    if status in {"ok", "empty"}:
        return termination is None
    if not isinstance(termination, dict) or set(termination) - {
        "code",
        "retryable",
        "detailCode",
    }:
        return False
    code = termination.get("code")
    retryable = termination.get("retryable")
    detail_code = termination.get("detailCode")
    expected = {
        "limitReached": ("limitReached", False),
        "timedOut": ({"deadlineExceeded", "dependencyPending"}, True),
        "unavailable": (
            {"unsupportedScope", "capacityExhausted", "providerUnavailable"},
            None,
        ),
        "failed": ("providerFailed", False),
    }.get(status)
    if expected is None or not isinstance(retryable, bool):
        return False
    expected_codes, expected_retryable = expected
    if isinstance(expected_codes, set):
        code_matches = code in expected_codes
    else:
        code_matches = code == expected_codes
    if not code_matches:
        return False
    if code == "capacityExhausted":
        expected_retryable = True
    elif status == "unavailable":
        expected_retryable = False
    if retryable is not expected_retryable:
        return False
    if code == "dependencyPending":
        return isinstance(detail_code, str) and bool(detail_code)
    return "detailCode" not in termination


def validate_code_search(scenario: dict[str, Any], payload: dict[str, Any] | None) -> None:
    if payload is None:
        return
    data = payload.get("data")
    sections = data.get("sections") if isinstance(data, dict) else None
    expected = ["semantic", "symbol", "lexical"]
    roles = (
        [section.get("role") for section in sections if isinstance(section, dict)]
        if isinstance(sections, list)
        else []
    )
    errors: list[str] = []
    if roles != expected:
        errors.append(
            "code search data.sections must contain exactly semantic, symbol, lexical in that order"
        )
    elif any(
        section.get("status")
        not in {"ok", "empty", "limitReached", "timedOut", "unavailable", "failed"}
        or not isinstance(section.get("provider"), str)
        or not isinstance(section.get("searchComplete"), bool)
        or section.get("ranking") not in {"provider", "none"}
        or section.get("ordering") not in {"provider", "providerTraversal"}
        or not isinstance(section.get("matches"), dict)
        or not isinstance(section.get("hits"), list)
        or not isinstance(section.get("diagnostics"), list)
        for section in sections
    ):
        errors.append(
            "code search role sections must expose status, completeness, ranking, count, hits, and diagnostics"
        )
    elif any(not valid_code_search_termination(section) for section in sections):
        errors.append(
            "code search role section termination must match status and retryability"
        )
    elif any(not valid_code_search_match_count(section) for section in sections):
        errors.append(
            "code search role section count must match status, hits, and exact/lowerBound/unknown relation"
        )
    elif sections[2].get("ranking") != "none" or sections[2].get("ordering") != "providerTraversal":
        errors.append("code search lexical role must be unranked in provider traversal order")
    if errors:
        scenario["status"] = "failed"
        scenario["errors"].extend(errors)


def indexed_code_search_state(payload: dict[str, Any] | None) -> str:
    """Classify how far the two indexed roles are from serving a search.

    A still-building index is a typed fact, not a phrase in the diagnostics:
    the role reports `timedOut` with a retryable `dependencyPending`
    termination. Reading the code rather than the prose is what keeps this
    poller from mistaking a pending index for a permanent failure.

    Both halves of that pair have to hold. `retryable` is what promises the
    next attempt can differ, so a payload carrying the code without it is
    invalid and waiting on it would only spend the deadline.
    """
    data = payload.get("data") if isinstance(payload, dict) else None
    sections = data.get("sections") if isinstance(data, dict) else None
    if not isinstance(sections, list):
        return "terminal"
    indexed = {
        section.get("role"): section
        for section in sections
        if isinstance(section, dict) and section.get("role") in INDEXED_SEARCH_ROLES
    }
    if set(indexed) != set(INDEXED_SEARCH_ROLES):
        return "terminal"
    if any(
        section.get("status") in {"ok", "empty", "limitReached"} for section in indexed.values()
    ):
        return "ready"
    # One role that can still become ready is reason enough to wait: the other
    # may have failed permanently and the search still succeeds once this one
    # finishes indexing.
    if any(
        section.get("status") == "timedOut"
        and isinstance(section.get("termination"), dict)
        and section["termination"].get("code") == "dependencyPending"
        and section["termination"].get("retryable") is True
        for section in indexed.values()
    ):
        return "building"
    return "terminal"


def describe_indexed_roles(payload: dict[str, Any] | None) -> str:
    """Name what each indexed role actually reported.

    The report keeps counts, not payloads, so a bare "no indexed provider
    became ready" leaves the next reader with nothing to act on: a role that
    is missing, one that is still building, and one whose binary is absent all
    produce the same sentence. Naming the status, the termination code and the
    first diagnostic is what makes the failure answerable from the artifact
    alone.
    """
    data = payload.get("data") if isinstance(payload, dict) else None
    sections = data.get("sections") if isinstance(data, dict) else None
    if not isinstance(sections, list):
        return "no sections were reported"
    described = []
    for section in sections:
        if not isinstance(section, dict) or section.get("role") not in INDEXED_SEARCH_ROLES:
            continue
        termination = section.get("termination")
        code = termination.get("code") if isinstance(termination, dict) else None
        diagnostics = section.get("diagnostics")
        detail = diagnostics[0] if isinstance(diagnostics, list) and diagnostics else ""
        described.append(
            f"{section.get('role')}={section.get('status')}"
            + (f"/{code}" if code else "")
            + (f" ({detail})" if detail else "")
        )
    return "; ".join(described) if described else "no indexed roles were reported"


def wait_for_indexed_code_search(
    run_attempt: Callable[[float], tuple[dict[str, Any], dict[str, Any] | None]],
    *,
    timeout_seconds: float,
    poll_interval_seconds: float = INDEX_POLL_INTERVAL_SECONDS,
    monotonic: Callable[[], float] = time.monotonic,
    sleep: Callable[[float], None] = time.sleep,
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    deadline = monotonic() + timeout_seconds
    attempts = 0
    total_duration_ms = 0
    last: tuple[dict[str, Any], dict[str, Any] | None] | None = None

    while True:
        remaining = deadline - monotonic()
        if remaining <= 0:
            if last is None:
                raise ValueError("indexed code search timeout must be positive")
            scenario, payload = last
            scenario["status"] = "failed"
            scenario["errors"].append(
                f"indexed code search did not become ready within {timeout_seconds:g} seconds"
            )
            scenario["durationMs"] = total_duration_ms
            scenario["metrics"] = {
                **scenario["metrics"],
                "indexAttempts": attempts,
                "indexedState": "building",
            }
            return scenario, payload

        scenario, payload = run_attempt(remaining)
        attempts += 1
        total_duration_ms += int(scenario.get("durationMs", 0))
        last = scenario, payload
        state = indexed_code_search_state(payload)
        if state == "ready":
            scenario["durationMs"] = total_duration_ms
            scenario["metrics"] = {
                **scenario["metrics"],
                "indexAttempts": attempts,
                "indexedState": state,
            }
            return scenario, payload
        if state == "terminal":
            scenario["status"] = "failed"
            scenario["errors"].append(
                "indexed code search terminated; no indexed provider became ready: "
                + describe_indexed_roles(payload)
            )
            scenario["durationMs"] = total_duration_ms
            scenario["metrics"] = {
                **scenario["metrics"],
                "indexAttempts": attempts,
                "indexedState": state,
            }
            return scenario, payload

        sleep_budget = deadline - monotonic()
        if sleep_budget <= 0:
            continue
        sleep(min(poll_interval_seconds, sleep_budget))


def relpath(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def first_existing(root: Path, patterns: list[str]) -> str | None:
    for pattern in patterns:
        matches = sorted(root.glob(pattern))
        if matches:
            return relpath(matches[0], root)
    return None


def template_kind(path: Path) -> str:
    text = path.read_text(encoding="utf-8", errors="ignore")[:8192]
    if "DataCompositionSchema" in text or "dataCompositionSchema" in text:
        return "dcs"
    if "SpreadsheetDocument" in text or "spreadsheet" in text.lower():
        return "mxl"
    return "unknown"


def optional_sample_scenarios(bsp_root: Path) -> list[tuple[str, str, str, dict[str, Any], bool]]:
    scenarios: list[tuple[str, str, str, dict[str, Any], bool]] = []
    form = first_existing(bsp_root, [f"{SOURCE_DIR}/**/Forms/*/Ext/Form.xml"])
    if form:
        scenarios.append(("form-info-sample", "Sample managed form info", "unica.form.info", {"FormPath": form, "Limit": 80}, True))
        scenarios.append(
            ("form-validate-sample", "Sample managed form validation", "unica.form.validate", {"FormPath": form, "MaxErrors": 30}, False)
        )

    role = first_existing(bsp_root, [f"{SOURCE_DIR}/Roles/*/Ext/Rights.xml"])
    if role:
        scenarios.append(("role-info-sample", "Sample role info", "unica.role.info", {"RightsPath": role, "Limit": 80}, True))
        scenarios.append(
            ("role-validate-sample", "Sample role validation", "unica.role.validate", {"RightsPath": role, "MaxErrors": 30}, False)
        )

    templates = sorted(bsp_root.glob(f"{SOURCE_DIR}/**/Templates/*/Ext/Template.xml"))
    dcs = next((path for path in templates if template_kind(path) == "dcs"), None)
    mxl = next((path for path in templates if template_kind(path) == "mxl"), None)
    if dcs:
        dcs_rel = relpath(dcs, bsp_root)
        scenarios.append(("dcs-info-sample", "Sample DCS info", "unica.dcs.info", {"TemplatePath": dcs_rel, "Limit": 80}, True))
        scenarios.append(
            ("dcs-validate-sample", "Sample DCS validation", "unica.dcs.validate", {"TemplatePath": dcs_rel, "MaxErrors": 30}, False)
        )
    if mxl:
        mxl_rel = relpath(mxl, bsp_root)
        scenarios.append(("mxl-info-sample", "Sample MXL info", "unica.mxl.info", {"TemplatePath": mxl_rel, "Limit": 80}, True))
        scenarios.append(
            ("mxl-validate-sample", "Sample MXL validation", "unica.mxl.validate", {"TemplatePath": mxl_rel, "MaxErrors": 30}, False)
        )
    return scenarios


def sample_bsl_path(bsp_root: Path) -> str | None:
    matches = sorted(bsp_root.glob(f"{SOURCE_DIR}/**/*.bsl"))
    return relpath(matches[0], bsp_root) if matches else None


def sample_bsl_search(bsp_root: Path) -> tuple[str, str] | None:
    for path in sorted(bsp_root.glob(f"{SOURCE_DIR}/**/*.bsl")):
        text = path.read_text(encoding="utf-8", errors="ignore")
        for query in ("Процедура", "Функция", "Экспорт"):
            if query in text:
                return relpath(path, bsp_root), query
    return None


def project_probe_scenarios() -> list[tuple[str, str, str, dict[str, Any], bool, bool]]:
    return [
        ("project-status", "Workspace status", "unica.project.status", {}, True, True),
        ("project-map", "Workspace source-set map", "unica.project.map", {}, True, True),
    ]


def base_tool_scenarios(
    bsp_root: Path, project_map_payload: dict[str, Any]
) -> list[tuple[str, str, str, dict[str, Any], bool, bool]]:
    source_set = project_source_set_name(project_map_payload, SOURCE_DIR)
    if source_set is None:
        raise ValueError(f"project map does not identify the source set for {SOURCE_DIR}")
    bsl_search = sample_bsl_search(bsp_root)
    code_search_args = {"sourceSet": source_set, "query": "Процедура", "limit": 20}
    if bsl_search:
        _, query = bsl_search
        code_search_args = {"sourceSet": source_set, "query": query, "limit": 20}

    scenarios: list[tuple[str, str, str, dict[str, Any], bool, bool]] = [
        ("cf-info", "BSP Configuration.xml overview", "unica.cf.info", {"ConfigPath": SOURCE_DIR}, True, True),
        ("cf-validate", "BSP Configuration.xml validation", "unica.cf.validate", {"ConfigPath": SOURCE_DIR, "MaxErrors": 50}, False, False),
        (
            "code-diagnostics-analyze",
            "BSL diagnostics source-set analysis",
            "unica.code.diagnostics",
            {"action": "analyze", "sourceSet": source_set, "limit": 100},
            False,
            False,
        ),
        (
            "code-search",
            "BSL indexed search smoke",
            "unica.code.search",
            code_search_args,
            True,
            True,
        ),
    ]
    bsl_path = sample_bsl_path(bsp_root)
    if bsl_path:
        scenarios.append(
            (
                "code-outline-sample",
                "Sample BSL module outline",
                "unica.code.outline",
                {"sourceDir": SOURCE_DIR, "path": bsl_path, "includeMethods": True},
                False,
                True,
            )
        )
    return scenarios


def extract_diagnostic_codes(payload: dict[str, Any] | None) -> list[str]:
    if not payload:
        return []
    codes: set[str] = set()
    # The public diagnostics contract exposes provider-neutral observations in
    # data.items.  Resource failures have their own error codes and must not be
    # reported as diagnostic findings.
    data = payload.get("data")
    if not isinstance(data, dict):
        return []
    items = data.get("items")
    if not isinstance(items, list):
        return []
    for item in items:
        if not isinstance(item, dict) or item.get("kind") != "diagnostic":
            continue
        code = item.get("code")
        if isinstance(code, str) and code:
            codes.add(code)
    return sorted(codes)


def summarize_cache(cache_dir: Path) -> dict[str, Any]:
    if not cache_dir.exists():
        return {"exists": False, "files": 0, "bytes": 0}
    files = [path for path in cache_dir.rglob("*") if path.is_file()]
    return {
        "exists": True,
        "files": len(files),
        "bytes": sum(path.stat().st_size for path in files),
    }


def build_summary(scenarios: list[dict[str, Any]], diagnostic_codes: list[str], cache_dir: Path) -> dict[str, Any]:
    blocking_failures = sum(
        1 for scenario in scenarios if scenario["blocking"] and scenario["status"] == "failed"
    )
    total_duration = sum(int(scenario["durationMs"]) for scenario in scenarios)
    return {
        "status": "failed" if blocking_failures else "passed",
        "blockingFailures": blocking_failures,
        "qualityFindings": {
            "diagnosticCodes": diagnostic_codes,
            "nonBlockingFailures": sum(
                1 for scenario in scenarios if not scenario["blocking"] and scenario["status"] == "failed"
            ),
        },
        "performance": {
            "totalDurationMs": total_duration,
            "scenarioCount": len(scenarios),
            "cache": summarize_cache(cache_dir),
        },
    }


def environment_metadata(
    run_unica: Path, runtime_overlay: list[str] | None = None
) -> dict[str, Any]:
    return {
        "os": platform.platform(),
        "python": platform.python_version(),
        "machine": platform.machine(),
        "runUnica": str(run_unica),
        "runtimeOverlay": runtime_overlay or [],
        "generatedAt": utc_now(),
    }


def build_assessment_report(
    *,
    run_unica: Path,
    bsp_root: Path,
    cache_dir: Path,
    out_dir: Path,
    release_tag: str,
    github_run_id: str,
    candidate_package: str,
    bsp_commit: str,
    timeout_seconds: int,
    bsp_ref: str = BSP_REF,
    runtime_overlay: list[str] | None = None,
) -> dict[str, Any]:
    scenarios: list[dict[str, Any]] = []
    diagnostic_codes: list[str] = []

    scenarios.append(run_tools_list_scenario(run_unica, bsp_root, cache_dir, timeout_seconds))

    project_map_payload: dict[str, Any] | None = None
    for scenario_id, title, tool, arguments, blocking, require_payload_ok in project_probe_scenarios():
        scenario, payload = run_tool_scenario(
            run_unica,
            bsp_root=bsp_root,
            cache_dir=cache_dir,
            scenario_id=scenario_id,
            title=title,
            tool=tool,
            arguments=arguments,
            timeout_seconds=timeout_seconds,
            blocking=blocking,
            require_payload_ok=require_payload_ok,
        )
        if scenario_id == "project-map":
            validate_project_map(scenario, payload)
            project_map_payload = payload
        scenarios.append(scenario)
        diagnostic_codes.extend(extract_diagnostic_codes(payload))

    try:
        base_scenarios = base_tool_scenarios(bsp_root, project_map_payload or {})
    except ValueError as error:
        scenarios.append(
            scenario_result(
                scenario_id="logical-source-set-resolution",
                title="Resolve BSP logical source set",
                tool="unica.project.map",
                arguments={"path": SOURCE_DIR},
                status="failed",
                duration_ms=0,
                blocking=True,
                errors=[str(error)],
            )
        )
        base_scenarios = []

    for scenario_id, title, tool, arguments, blocking, require_payload_ok in base_scenarios:
        def run_attempt(attempt_timeout_seconds: float) -> tuple[dict[str, Any], dict[str, Any] | None]:
            return run_tool_scenario(
                run_unica,
                bsp_root=bsp_root,
                cache_dir=cache_dir,
                scenario_id=scenario_id,
                title=title,
                tool=tool,
                arguments=arguments,
                timeout_seconds=attempt_timeout_seconds,
                blocking=blocking,
                require_payload_ok=require_payload_ok,
            )

        # A fresh BSP has no index yet, so the search scenario is allowed to
        # retry until one indexed role is ready. Every attempt draws from the
        # one readiness deadline, so retrying cannot extend the assessment by
        # another full per-attempt timeout.
        if scenario_id == "code-search":
            scenario, payload = wait_for_indexed_code_search(
                run_attempt,
                timeout_seconds=min(timeout_seconds, INDEX_WAIT_TIMEOUT_SECONDS),
            )
            validate_code_search(scenario, payload)
        else:
            scenario, payload = run_attempt(float(timeout_seconds))
        scenarios.append(scenario)
        diagnostic_codes.extend(extract_diagnostic_codes(payload))

    for scenario_id, title, tool, arguments, require_payload_ok in optional_sample_scenarios(bsp_root):
        scenario, payload = run_tool_scenario(
            run_unica,
            bsp_root=bsp_root,
            cache_dir=cache_dir,
            scenario_id=scenario_id,
            title=title,
            tool=tool,
            arguments=arguments,
            timeout_seconds=timeout_seconds,
            blocking=False,
            require_payload_ok=require_payload_ok,
        )
        scenarios.append(scenario)
        diagnostic_codes.extend(extract_diagnostic_codes(payload))

    diagnostic_codes = sorted(set(diagnostic_codes))[:20]
    if diagnostic_codes:
        scenario, _payload = run_tool_scenario(
            run_unica,
            bsp_root=bsp_root,
            cache_dir=cache_dir,
            scenario_id="standards-explain-diagnostics",
            title="Explain top diagnostic codes through standards adapter",
            tool="unica.standards.explain",
            arguments={"codes": diagnostic_codes[:10]},
            timeout_seconds=timeout_seconds,
            blocking=False,
            require_payload_ok=True,
        )
        scenarios.append(scenario)

    description = read_json(bsp_root / "description.json")
    report = {
        "schemaVersion": SCHEMA_VERSION,
        "unicaVersion": unica_version(run_unica),
        "releaseTag": release_tag,
        "githubRunId": github_run_id,
        "candidatePackage": candidate_package,
        "bsp": {
            "repo": BSP_REPO,
            "ref": bsp_ref,
            "requestedRef": bsp_ref,
            "commit": bsp_commit,
            "descriptionVersion": description.get("Версия"),
            "descriptionDate": description.get("Дата"),
        },
        "environment": environment_metadata(run_unica, runtime_overlay),
        "scenarios": scenarios,
        "summary": build_summary(scenarios, diagnostic_codes, cache_dir),
    }
    write_report_files(report, out_dir)
    return report


def render_html(report: dict[str, Any]) -> str:
    status = html.escape(str(report["summary"]["status"]))
    release = html.escape(str(report["releaseTag"]))
    bsp_commit = html.escape(str(report["bsp"]["commit"]))
    blocking = html.escape(str(report["summary"]["blockingFailures"]))
    rows = []
    for scenario in report["scenarios"]:
        errors = "<br>".join(html.escape(error) for error in scenario.get("errors", []))
        rows.append(
            "<tr>"
            f"<td>{html.escape(str(scenario['id']))}</td>"
            f"<td>{html.escape(str(scenario['title']))}</td>"
            f"<td>{html.escape(str(scenario['tool']))}</td>"
            f"<td>{html.escape(str(scenario['status']))}</td>"
            f"<td>{html.escape(str(scenario['durationMs']))}</td>"
            f"<td>{html.escape(str(scenario['blocking']))}</td>"
            f"<td>{errors}</td>"
            "</tr>"
        )
    codes = ", ".join(html.escape(code) for code in report["summary"]["qualityFindings"].get("diagnosticCodes", []))
    return "\n".join(
        [
            "<!doctype html>",
            '<html lang="en">',
            "<head>",
            '<meta charset="utf-8">',
            "<title>Unica Release Assessment</title>",
            "<style>",
            "body{font-family:-apple-system,BlinkMacSystemFont,Segoe UI,sans-serif;margin:32px;line-height:1.45}",
            "table{border-collapse:collapse;width:100%;font-size:14px}",
            "th,td{border:1px solid #d0d7de;padding:6px 8px;text-align:left;vertical-align:top}",
            "th{background:#f6f8fa}",
            ".passed{color:#116329}.failed{color:#cf222e}",
            "</style>",
            "</head>",
            "<body>",
            f"<h1>Unica Release Assessment {release}</h1>",
            f'<p>Status: <strong class="{status}">{status}</strong></p>',
            f"<p>Blocking failures: {blocking}</p>",
            f"<p>BSP commit: <code>{bsp_commit}</code></p>",
            f"<p>Diagnostic codes: {codes or 'none'}</p>",
            "<h2>Scenarios</h2>",
            "<table>",
            "<thead><tr><th>ID</th><th>Title</th><th>Tool</th><th>Status</th><th>Duration ms</th><th>Blocking</th><th>Errors</th></tr></thead>",
            "<tbody>",
            *rows,
            "</tbody>",
            "</table>",
            '<p><a href="assessment.json">assessment.json</a> | <a href="assessment.ndjson">assessment.ndjson</a> | <a href="summary.md">summary.md</a></p>',
            "</body>",
            "</html>",
            "",
        ]
    )


def render_summary_markdown(report: dict[str, Any]) -> str:
    lines = [
        f"# Unica Release Assessment {report['releaseTag']}",
        "",
        f"- Status: `{report['summary']['status']}`",
        f"- Blocking failures: `{report['summary']['blockingFailures']}`",
        f"- BSP commit: `{report['bsp']['commit']}`",
        f"- BSP version: `{report['bsp'].get('descriptionVersion')}`",
        f"- Total duration: `{report['summary']['performance']['totalDurationMs']} ms`",
        "",
        "## Scenarios",
        "",
    ]
    for scenario in report["scenarios"]:
        lines.append(
            f"- `{scenario['status']}` `{scenario['id']}` ({scenario['durationMs']} ms, blocking={scenario['blocking']})"
        )
    lines.append("")
    return "\n".join(lines)


def write_report_files(report: dict[str, Any], out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "assessment.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    with (out_dir / "assessment.ndjson").open("w", encoding="utf-8") as stream:
        for scenario in report["scenarios"]:
            stream.write(json.dumps(scenario, ensure_ascii=False, sort_keys=True) + "\n")
    (out_dir / "index.html").write_text(render_html(report), encoding="utf-8")
    (out_dir / "summary.md").write_text(render_summary_markdown(report), encoding="utf-8")


def print_blocking_failure_summary(report: dict[str, Any]) -> None:
    failures = [
        scenario
        for scenario in report.get("scenarios", [])
        if scenario.get("blocking") and scenario.get("status") == "failed"
    ]
    if not failures:
        return
    print("blocking assessment failures:", file=sys.stderr)
    for scenario in failures:
        errors = "; ".join(str(error) for error in scenario.get("errors", [])) or "no error details"
        print(f"- {scenario.get('id')}: {errors}", file=sys.stderr)


def copytree_replace(source: Path, target: Path) -> None:
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target)


def copy_versioned_pages(report_dir: Path, pages_root: Path, release_tag: str) -> None:
    assessments = pages_root / "assessments"
    assessments.mkdir(parents=True, exist_ok=True)
    copytree_replace(report_dir, assessments / release_tag)
    copytree_replace(report_dir, assessments / "latest")
    index = assessments / "index.html"
    if not index.exists():
        index.write_text(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Unica Assessments</title></head>"
            "<body><h1>Unica Assessments</h1><p>Open a versioned assessment folder.</p></body></html>\n",
            encoding="utf-8",
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--package-archive", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, default=Path(".build/release-assessment/work"))
    parser.add_argument("--out-dir", type=Path, default=Path("dist/release-assessment"))
    parser.add_argument("--pages-root", type=Path)
    parser.add_argument("--release-tag", default=release_tag_from_env())
    parser.add_argument("--github-run-id", default=os.environ.get("GITHUB_RUN_ID", "local"))
    parser.add_argument("--bsp-ref", default=BSP_REF)
    parser.add_argument("--timeout-seconds", type=int, default=600)
    parser.add_argument("--engine-overlay", type=Path)
    args = parser.parse_args()

    work_dir = args.work_dir.resolve()
    package_archive = args.package_archive.resolve()
    run_unica = extract_marketplace_archive(package_archive, work_dir / "marketplace")
    runtime_overlay = (
        overlay_runtime_files(
            run_unica,
            prepare_runtime_overlay(
                args.engine_overlay.resolve(),
                work_dir / "engine-overlay",
            ),
        )
        if args.engine_overlay
        else []
    )
    bsp_root, bsp_commit = download_bsp(work_dir / "bsp", ref=args.bsp_ref)
    report = build_assessment_report(
        run_unica=run_unica,
        bsp_root=bsp_root,
        cache_dir=work_dir / "cache",
        out_dir=args.out_dir,
        release_tag=args.release_tag,
        github_run_id=args.github_run_id,
        candidate_package=package_archive.name,
        bsp_commit=bsp_commit,
        bsp_ref=args.bsp_ref,
        timeout_seconds=args.timeout_seconds,
        runtime_overlay=runtime_overlay,
    )
    if args.pages_root:
        copy_versioned_pages(args.out_dir, args.pages_root, args.release_tag)
    if report["summary"]["blockingFailures"]:
        print_blocking_failure_summary(report)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
