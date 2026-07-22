#!/usr/bin/env python3
"""Run a packaged Unica binary through MCP discovery acceptance."""

from __future__ import annotations

import argparse
import json
import os
import queue
import shutil
import subprocess
import tempfile
import threading
import time
from pathlib import Path
from typing import Any, TextIO


REQUIRED_DCS_TOOLS = {
    "unica.dcs.compile",
    "unica.dcs.edit",
    "unica.dcs.info",
    "unica.dcs.validate",
}
REQUIRED_TOOLS = REQUIRED_DCS_TOOLS | {
    "unica.project.status",
    "unica.project.discover",
    "unica.standards.search",
    "unica.standards.explain",
}
REMOVED_DCS_TOOL_ALIASES = {
    name.replace(".dcs.", ".s" + "kd.") for name in REQUIRED_DCS_TOOLS
}
DISCOVERY_FIXTURE = (
    Path(__file__).resolve().parents[2]
    / "tests"
    / "fixtures"
    / "extension-point-discovery"
    / "ut115"
)
DISCOVERY_TASK = "При поступлении товаров контролировать остаточный срок годности серий"
REQUIRED_DISCOVERY_TARGETS = {
    "Document.ПриобретениеТоваровУслуг.TabularSection.Серии",
    "DataProcessor.ПодборСерийВДокументы",
    (
        "DataProcessor.ПодборСерийВДокументы.Form."
        "РегистрацияИПодборСерийПоОднойСтрокеТоваров"
    ),
}
ALLOWED_RECOMMENDATION_BASES = {
    "metadata_structure",
    "managed_form_binding",
    "proven_runtime_flow",
}


def read_stdout(stream: TextIO, events: queue.Queue[tuple[str, str]]) -> None:
    try:
        for line in stream:
            events.put(("line", line))
    finally:
        events.put(("eof", ""))


def read_stderr(stream: TextIO, lines: list[str]) -> None:
    lines.extend(stream.readlines())


def send_message(process: subprocess.Popen[str], message: dict[str, Any]) -> None:
    if process.stdin is None:
        raise SystemExit("Unica MCP stdin is unavailable")
    try:
        process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        process.stdin.flush()
    except (BrokenPipeError, OSError) as error:
        raise SystemExit(f"Unica MCP closed stdin before responding: {error}") from error


def collect_responses(
    events: queue.Queue[tuple[str, str]],
    responses: dict[int, dict[str, Any]],
    expected_ids: set[int],
    deadline: float,
) -> None:
    while not expected_ids.issubset(responses):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            missing = sorted(expected_ids - responses.keys())
            raise SystemExit(f"Unica MCP smoke timed out waiting for response ids: {missing}")
        try:
            event, line = events.get(timeout=remaining)
        except queue.Empty as error:
            missing = sorted(expected_ids - responses.keys())
            raise SystemExit(
                f"Unica MCP smoke timed out waiting for response ids: {missing}"
            ) from error
        if event == "eof":
            missing = sorted(expected_ids - responses.keys())
            raise SystemExit(f"Unica MCP stdout closed before response ids: {missing}")
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise SystemExit(f"Unica MCP emitted invalid JSON: {error}: {line}") from error
        if isinstance(value, dict) and isinstance(value.get("id"), int):
            responses[value["id"]] = value


def validate_initialize_and_tools(responses: dict[int, dict[str, Any]]) -> None:
    if "result" not in responses.get(1, {}):
        raise SystemExit("Unica MCP initialize response is missing")
    tools = responses.get(2, {}).get("result", {}).get("tools")
    if not isinstance(tools, list):
        raise SystemExit("Unica MCP tools/list response is missing")
    names = {
        tool.get("name")
        for tool in tools
        if isinstance(tool, dict) and isinstance(tool.get("name"), str)
    }
    missing = sorted(REQUIRED_TOOLS - names)
    if missing:
        raise SystemExit(f"Unica MCP tools/list is missing: {', '.join(missing)}")
    removed_aliases = sorted(REMOVED_DCS_TOOL_ALIASES & names)
    if removed_aliases:
        raise SystemExit(
            f"Unica MCP tools/list exposes removed DCS aliases: {', '.join(removed_aliases)}"
        )


def operation_payload(response: dict[str, Any]) -> dict[str, Any]:
    content = response.get("result", {}).get("content")
    if not isinstance(content, list) or not content:
        raise SystemExit("Unica MCP discovery response content is missing")
    first = content[0]
    if not isinstance(first, dict) or not isinstance(first.get("text"), str):
        raise SystemExit("Unica MCP discovery response text is missing")
    try:
        payload = json.loads(first["text"])
    except json.JSONDecodeError as error:
        raise SystemExit(f"Unica MCP discovery response text is invalid JSON: {error}") from error
    if not isinstance(payload, dict):
        raise SystemExit("Unica MCP discovery operation result must be an object")
    if payload.get("ok") is not True:
        raise SystemExit("Unica MCP discovery OperationResult.ok must be true")
    return payload


def validate_discovery(response: dict[str, Any]) -> None:
    payload = operation_payload(response)
    data = payload.get("data")
    if not isinstance(data, dict) or "discovery" not in data:
        raise SystemExit("Unica MCP data.discovery is missing")
    discovery = data["discovery"]
    if not isinstance(discovery, dict):
        raise SystemExit("Unica MCP data.discovery must be an object")
    if discovery.get("status") != "partial":
        raise SystemExit("Unica MCP task-only discovery must report partial")

    candidates = discovery.get("candidates")
    if not isinstance(candidates, list):
        raise SystemExit("Unica MCP task-only discovery candidates are missing")
    candidate_targets = {
        candidate.get("target")
        for candidate in candidates
        if isinstance(candidate, dict) and isinstance(candidate.get("target"), str)
    }
    missing_targets = sorted(REQUIRED_DISCOVERY_TARGETS - candidate_targets)
    if missing_targets:
        raise SystemExit(
            "Unica MCP task-only discovery is missing candidates: "
            + ", ".join(missing_targets)
        )
    for candidate in candidates:
        if not isinstance(candidate, dict):
            raise SystemExit("Unica MCP discovery candidate must be an object")
        recommendation = candidate.get("recommendation")
        if not isinstance(recommendation, dict):
            raise SystemExit("Unica MCP discovery candidate recommendation is missing")
        summary = recommendation.get("summary")
        basis = recommendation.get("basis")
        if not isinstance(summary, str) or not summary.strip():
            raise SystemExit("Unica MCP discovery candidate recommendation summary is missing")
        if (
            not isinstance(basis, list)
            or not basis
            or any(
                not isinstance(item, str)
                or item not in ALLOWED_RECOMMENDATION_BASES
                for item in basis
            )
            or len(set(basis)) != len(basis)
        ):
            raise SystemExit("Unica MCP discovery candidate recommendation basis is invalid")

    warnings = discovery.get("warnings")
    if not isinstance(warnings, list) or not any(
        isinstance(warning, dict)
        and warning.get("blocking") is True
        and warning.get("code") == "alternative_relevant_tabular_section"
        for warning in warnings
    ):
        raise SystemExit(
            "Unica MCP task-only discovery "
            "alternative_relevant_tabular_section blocking warning is missing"
        )

    missing_checks = discovery.get("missingChecks")
    missing_codes = {
        check.get("code")
        for check in missing_checks or []
        if isinstance(check, dict) and isinstance(check.get("code"), str)
    }
    if "bsl_index_missing" not in missing_codes:
        raise SystemExit("Unica MCP task-only discovery bsl_index_missing check is missing")


def close_process(process: subprocess.Popen[str], timeout_seconds: float) -> None:
    if process.stdin is not None and not process.stdin.closed:
        try:
            process.stdin.close()
        except (BrokenPipeError, OSError):
            pass
    try:
        process.wait(timeout=max(timeout_seconds, 0.1))
    except subprocess.TimeoutExpired:
        process.terminate()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=1)


def run_protocol(
    command: list[str],
    plugin_root: Path,
    workspace: Path,
    timeout_seconds: float,
) -> None:
    environment = os.environ.copy()
    environment["UNICA_PLUGIN_ROOT"] = str(plugin_root.resolve())
    environment["UNICA_CACHE_DIR"] = str((workspace / ".unica-smoke-cache").resolve())
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="strict",
            cwd=workspace,
            env=environment,
        )
    except OSError as error:
        raise SystemExit(f"failed to start Unica MCP: {error}") from error
    if process.stdout is None or process.stderr is None:
        close_process(process, timeout_seconds)
        raise SystemExit("Unica MCP output pipes are unavailable")

    events: queue.Queue[tuple[str, str]] = queue.Queue()
    stderr_lines: list[str] = []
    stdout_thread = threading.Thread(
        target=read_stdout, args=(process.stdout, events), daemon=True
    )
    stderr_thread = threading.Thread(
        target=read_stderr, args=(process.stderr, stderr_lines), daemon=True
    )
    stdout_thread.start()
    stderr_thread.start()
    deadline = time.monotonic() + timeout_seconds
    responses: dict[int, dict[str, Any]] = {}
    protocol_failure: SystemExit | None = None

    try:
        send_message(
            process,
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "unica-release-smoke", "version": "1"},
                },
            },
        )
        send_message(
            process,
            {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        )
        send_message(
            process,
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        )
        collect_responses(events, responses, {1, 2}, deadline)
        validate_initialize_and_tools(responses)

        send_message(
            process,
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "unica.project.discover",
                    "arguments": {"mode": "explore", "task": DISCOVERY_TASK},
                },
            },
        )
        collect_responses(events, responses, {3}, deadline)
        validate_discovery(responses[3])
    except SystemExit as error:
        protocol_failure = error
    finally:
        remaining = max(deadline - time.monotonic(), 0.1)
        close_process(process, remaining)
        stdout_thread.join(timeout=1)
        stderr_thread.join(timeout=1)

    detail = "".join(stderr_lines).strip()
    if protocol_failure is not None:
        failure_context: list[str] = []
        if process.returncode != 0:
            failure_context.append(f"Unica MCP exited with {process.returncode}")
        if detail:
            failure_context.append(f"stderr: {detail}")
        if failure_context:
            raise SystemExit(
                f"{protocol_failure}; {'; '.join(failure_context)}"
            ) from None
        raise protocol_failure
    if process.returncode != 0:
        raise SystemExit(
            f"Unica MCP exited with {process.returncode}: {detail or 'no process output'}"
        )


def smoke(command: list[str], plugin_root: Path, timeout_seconds: float) -> None:
    fixture_source = DISCOVERY_FIXTURE / "src"
    fixture_manifest = DISCOVERY_FIXTURE / "v8project.yaml"
    if not fixture_source.is_dir() or not fixture_manifest.is_file():
        raise SystemExit(f"Unica discovery smoke fixture is missing: {DISCOVERY_FIXTURE}")
    with tempfile.TemporaryDirectory(prefix="unica-discovery-smoke-") as directory:
        workspace = Path(directory)
        shutil.copytree(fixture_source, workspace / "src")
        shutil.copy2(fixture_manifest, workspace / "v8project.yaml")
        run_protocol(command, plugin_root, workspace, timeout_seconds)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--binary-arg", action="append", default=[])
    parser.add_argument("--plugin-root", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=float, default=20)
    args = parser.parse_args()
    binary = Path(args.binary)
    executable = str(binary.resolve()) if binary.exists() else args.binary
    smoke([executable, *args.binary_arg], args.plugin_root, args.timeout_seconds)
    print("verified Unica MCP task-only discovery")


if __name__ == "__main__":
    main()
