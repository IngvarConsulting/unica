#!/usr/bin/env python3
"""Run a packaged Unica binary through MCP initialize and tools/list."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path


REQUIRED_DCS_TOOLS = {
    "unica.dcs.compile",
    "unica.dcs.edit",
    "unica.dcs.info",
    "unica.dcs.validate",
}
REQUIRED_TOOLS = REQUIRED_DCS_TOOLS | {
    "unica.project.status",
    "unica.standards.search",
    "unica.standards.explain",
}
REMOVED_DCS_TOOL_ALIASES = {
    name.replace(".dcs.", ".s" + "kd.") for name in REQUIRED_DCS_TOOLS
}


def smoke(command: list[str], plugin_root: Path, timeout_seconds: float) -> None:
    messages = [
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
        {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
    ]
    request = "".join(json.dumps(message, separators=(",", ":")) + "\n" for message in messages)
    environment = os.environ.copy()
    environment["UNICA_PLUGIN_ROOT"] = str(plugin_root.resolve())
    try:
        result = subprocess.run(
            command,
            input=request,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
            check=False,
            env=environment,
        )
    except subprocess.TimeoutExpired as error:
        raise SystemExit(f"Unica MCP smoke timed out after {timeout_seconds:g}s") from error
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "no process output"
        raise SystemExit(f"Unica MCP exited with {result.returncode}: {detail}")

    responses: dict[int, dict] = {}
    for line in result.stdout.splitlines():
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise SystemExit(f"Unica MCP emitted invalid JSON: {error}: {line}") from error
        if isinstance(value, dict) and isinstance(value.get("id"), int):
            responses[value["id"]] = value

    initialize = responses.get(1, {})
    if "result" not in initialize:
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


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--binary-arg", action="append", default=[])
    parser.add_argument("--plugin-root", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=float, default=20)
    args = parser.parse_args()
    smoke([args.binary, *args.binary_arg], args.plugin_root, args.timeout_seconds)
    print("verified Unica MCP initialize and tools/list")


if __name__ == "__main__":
    main()
