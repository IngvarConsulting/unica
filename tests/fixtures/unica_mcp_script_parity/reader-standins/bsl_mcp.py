#!/usr/bin/env python3
"""Deterministic stdio MCP stand-in for bsl-analyzer and rlm-tools-bsl."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path


def log_call(tool: str, arguments: dict[str, object]) -> None:
    path = os.environ.get("UNICA_READER_STANDIN_LOG")
    if not path:
        return
    with Path(path).open("a", encoding="utf-8") as stream:
        stream.write(json.dumps({"tool": tool, "arguments": arguments}, ensure_ascii=False) + "\n")


def text_result(request_id: object, payload: object) -> dict[str, object]:
    text = payload if isinstance(payload, str) else json.dumps(payload, ensure_ascii=False)
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "result": {"content": [{"type": "text", "text": text}]},
    }


def tool_payload(tool: str, arguments: dict[str, object]) -> object:
    log_call(tool, arguments)
    if tool == "search":
        return "No results found."
    if tool == "graph":
        return {"nodes": [], "edges": [], "truncated": False}
    if tool == "diagnostics":
        return {"findings": [], "truncated": False}
    if tool == "rlm_start":
        return {"session_id": "reader-parity", "index": {"index_status": "ready"}}
    if tool == "rlm_execute":
        code = str(arguments.get("code", ""))
        if "find_definition" in code:
            stdout: object = {
                "definitions": [
                    {
                        "file": "CommonModules/ParitySearch/Ext/Module.bsl",
                        "line": 1,
                        "kind": "procedure",
                        "name": "ОбработкаПроведения",
                        "params": [],
                        "export": True,
                    }
                ]
            }
        else:
            stdout = []
        return {"stdout": json.dumps(stdout, ensure_ascii=False), "stderr": ""}
    if tool == "rlm_end":
        return {"closed": True}
    return {"error": f"unsupported reader stand-in tool: {tool}"}


def main() -> int:
    for raw_line in sys.stdin:
        request = json.loads(raw_line)
        request_id = request.get("id")
        method = request.get("method")
        if request_id is None:
            continue
        if method == "initialize":
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "unica-reader-standin", "version": "1"},
                },
            }
        elif method == "tools/list":
            response = {"jsonrpc": "2.0", "id": request_id, "result": {"tools": []}}
        elif method == "tools/call":
            params = request.get("params", {})
            tool = str(params.get("name", ""))
            arguments = params.get("arguments", {})
            response = text_result(request_id, tool_payload(tool, arguments))
        else:
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {"code": -32601, "message": f"unsupported method: {method}"},
            }
        print(json.dumps(response, ensure_ascii=False), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
