#!/usr/bin/env python3
"""Deterministic stdio MCP stand-in for bsl-analyzer and rlm-bsl-mcp."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
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
    mode = os.environ.get("UNICA_RLM_CONTRACT_STANDIN_MODE", "")
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
            if mode == "definition_list":
                stdout = []
            else:
                params: object = []
                if mode == "string_params":
                    params = "Argument"
                elif mode:
                    params = ["Argument"]
                stdout = {
                    "definitions": [
                        {
                            "file": "CommonModules/ParitySearch/Ext/Module.bsl",
                            "line": 1,
                            "kind": "procedure",
                            "name": "ОбработкаПроведения",
                            "params": params,
                            "export": True,
                        }
                    ],
                    "_meta": {"total_is_lower_bound": False},
                }
        elif "get_object_profile" in code and mode == "profile_list":
            stdout = []
        elif "get_object_profile" in code and mode:
            stdout = {
                "object_name": "ContractOne",
                "sections": {},
                "_meta": {"truncated": False},
            }
        elif mode == "search_object":
            stdout = {}
        elif mode:
            metadata: object = {"truncated": False}
            if mode == "string_boolean":
                metadata = {"truncated": "false"}
            elif mode == "scalar_metadata":
                metadata = "invalid"
            stdout = [{"text": "ContractTest1", "_meta": metadata}]
        else:
            stdout = []
        return {"stdout": json.dumps(stdout, ensure_ascii=False), "stderr": ""}
    if tool == "rlm_end":
        return {"closed": True}
    return {"error": f"unsupported reader stand-in tool: {tool}"}


def main() -> int:
    mode = os.environ.get("UNICA_RLM_CONTRACT_STANDIN_MODE", "")
    if mode == "hang":
        child = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(30)"],
        )
        Path(os.environ["UNICA_RLM_CONTRACT_PID_LOG"]).write_text(
            f"{os.getpid()}\n{child.pid}\n",
            encoding="utf-8",
        )
    for raw_line in sys.stdin:
        request = json.loads(raw_line)
        rpc_log = os.environ.get("UNICA_RLM_CONTRACT_RPC_LOG")
        if rpc_log:
            with Path(rpc_log).open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(request, ensure_ascii=False) + "\n")
        if mode == "hang":
            time.sleep(30)
            continue
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
