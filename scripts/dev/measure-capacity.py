#!/usr/bin/env python3
"""Time public Unica calls on one workspace and read capacity observations.

    measure-capacity.py --binary target/release/unica --workspace DIR [--rounds 2]

The server runs with a private provider state directory. Each call is sent
over MCP stdio exactly as a host sends it; a call handed off to a Task is
followed with ``unica.task.result`` until it ends, and its time includes the
wait. After the session the daemon exits on its idle grace, flushes its
capacity snapshot, and ``unica --capacity-report`` reads it back. The report
is JSON on stdout: per call the wall time, outcome, refusal code and response
size, plus the capacity snapshot (peak daemon RSS, find and view pressure).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

MARKER = "СинтетическаяМеткаПоиска"


class Server:
    def __init__(self, binary: Path, workspace: Path, state: Path):
        env = dict(os.environ)
        env["UNICA_PROVIDER_STATE_DIR"] = str(state)
        env["UNICA_DAEMON_IDLE_GRACE_MS"] = "2000"
        self.stderr = open(state / "stderr.log", "wb")  # noqa: SIM115 - lives with the process
        self.process = subprocess.Popen(
            [str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=self.stderr, cwd=workspace, env=env,
        )
        self.next_id = 0
        self.request("initialize", {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "measure-capacity", "version": "0"},
        })
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, payload) -> None:
        self.process.stdin.write((json.dumps(payload, ensure_ascii=False) + "\n").encode())
        self.process.stdin.flush()

    def request(self, method: str, params) -> tuple[dict, int]:
        self.next_id += 1
        ident = self.next_id
        self.send({"jsonrpc": "2.0", "id": ident, "method": method, "params": params})
        while True:
            line = self.process.stdout.readline()
            if not line:
                raise RuntimeError("unica closed its output")
            payload = json.loads(line)
            if payload.get("id") == ident:
                return payload, len(line)

    def call(self, tool: str, arguments) -> tuple[dict, int]:
        return self.request("tools/call", {"name": tool, "arguments": arguments})

    def close(self) -> None:
        self.process.stdin.close()
        self.process.wait(timeout=120)
        self.stderr.close()


def outcome(payload: dict) -> tuple[str, str | None, dict]:
    result = payload.get("result") or {}
    content = result.get("structuredContent") or {}
    if "error" in payload:
        return "rpc_error", str(payload["error"].get("code")), content
    if content.get("ok"):
        return "ok", None, content
    diagnostics = content.get("diagnostics") or [{}]
    return "refused", diagnostics[0].get("code"), content


def task_id(content: dict) -> str | None:
    data = content.get("data") or {}
    task = data.get("task") if isinstance(data, dict) else None
    if isinstance(task, dict) and task.get("taskId"):
        return task["taskId"]
    for action in content.get("next") or []:
        args = action.get("args") or {}
        if action.get("tool") == "unica.task.result" and args.get("taskId"):
            return args["taskId"]
    return None


def timed(server: Server, tool: str, arguments) -> dict:
    started = time.monotonic()
    payload, size = server.call(tool, arguments)
    state, code, content = outcome(payload)
    handed = task_id(content)
    if handed:
        while True:
            payload, size = server.call("unica.task.result", {"taskId": handed, "waitMs": 7000})
            state, code, content = outcome(payload)
            data = content.get("data") or {}
            task = data.get("task") if isinstance(data, dict) else None
            if not (isinstance(task, dict) and task.get("status") in {"working", "queued"}):
                break
    return {
        "tool": tool,
        "args": arguments,
        "seconds": round(time.monotonic() - started, 3),
        "outcome": state,
        "code": code,
        "task": bool(handed),
        "responseBytes": size,
        "summary": (content.get("summary") or "")[:200],
    }


def plan(workspace: Path) -> list[tuple[str, dict]]:
    def first(directory: str) -> str | None:
        return next((p.stem for p in sorted((workspace / "src" / directory).glob("*.xml"))), None)

    module = first("CommonModules")
    catalog = first("Catalogs")
    absent = "ОтсутствующаяВКорпусеСтрока"
    return [
        ("unica.view", {}),
        ("unica.view", {"at": "main:Configuration"}),
        ("unica.view", {"at": "main:CommonModule"}),
        ("unica.view", {"at": f"main:CommonModule.{module}"}),
        ("unica.view", {"at": f"main:Catalog.{catalog}"}),
        # A lexical search for an absent string walks the whole corpus.
        ("unica.search", {"query": absent, "role": "lexical"}),
        ("unica.search", {"query": MARKER, "role": "lexical", "limit": 50}),
        ("unica.search", {"query": absent}),
        ("unica.search", {"query": MARKER, "limit": 50}),
        ("unica.search", {"query": "Справочник00042", "corpus": "names"}),
        ("unica.check", {}),
    ]


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=2)
    args = parser.parse_args(argv)
    binary = args.binary.resolve()
    workspace = args.workspace.resolve()
    with tempfile.TemporaryDirectory(prefix="unica-capacity-") as state_dir:
        # The daemon refuses a state root reached through a symbolic link,
        # and the system temporary directory is one on macOS.
        state = Path(state_dir).resolve()
        server = Server(binary, workspace, state)
        rounds = []
        try:
            for _ in range(args.rounds):
                rounds.append([timed(server, tool, arguments) for tool, arguments in plan(workspace)])
        except RuntimeError:
            server.stderr.flush()
            sys.stderr.write((state / "stderr.log").read_text(errors="replace")[-3000:])
            raise
        finally:
            server.close()
        # The daemon flushes its snapshot when it exits on the idle grace.
        time.sleep(5)
        env = dict(os.environ, UNICA_PROVIDER_STATE_DIR=str(state))
        report = subprocess.run(
            [str(binary), "--capacity-report"], capture_output=True, text=True, env=env, check=False
        )
        try:
            capacity = json.loads(report.stdout) if report.stdout.strip() else None
        except json.JSONDecodeError:
            capacity = {"raw": report.stdout[:2000]}
        print(json.dumps({"workspace": str(workspace), "rounds": rounds, "capacity": capacity},
                         ensure_ascii=False, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
