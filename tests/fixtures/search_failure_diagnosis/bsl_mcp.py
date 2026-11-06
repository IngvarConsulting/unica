#!/usr/bin/env python3
"""External analyzer stand-in; never implements the public Unica protocol."""
from __future__ import annotations

import json
import os
import sys
import threading
from pathlib import Path


CANARIES = [
    "/private/ClientCase2470/source/Module.bsl",
    "CompanyPrivate2470",
    "TokenPrivate2470_a9e7",
    "QueryPrivate2470",
    "StderrPrivate2470",
]


def main():
    mode = os.environ["UNICA_SEARCH_FAILURE_FIXTURE_MODE"]
    for line in sys.stdin:
        request = json.loads(line)
        if "id" not in request:
            continue
        response = {"jsonrpc": "2.0", "id": request["id"]}
        if request["method"] == "initialize":
            response["result"] = {
                "protocolVersion": "2025-03-26", "capabilities": {"tools": {}},
                "serverInfo": {"name": "search-diagnosis-external-fixture", "version": "1"},
            }
        elif request["method"] == "tools/list":
            response["result"] = {"tools": []}
        elif request["method"] == "tools/call":
            params = request.get("params", {})
            with Path(os.environ["UNICA_READER_STANDIN_LOG"]).open("a", encoding="utf-8") as log:
                log.write(json.dumps(params) + "\n")
            print(" ".join(CANARIES), file=sys.stderr, flush=True)
            if mode == "connection":
                return 23
            if mode == "timeout":
                # The caller's explicit provider deadline ends the request;
                # the owned-process cleanup must stop this unresponsive child.
                threading.Event().wait(120)
                return 24
            if mode == "blocked-unknown-json":
                # Only the test, after observing the real public Task ID,
                # releases this response. No timing guess creates the Task.
                with Path(os.environ["UNICA_SEARCH_FAILURE_RELEASE"]).open("rb", buffering=0) as release:
                    if release.read(1) != b"1":
                        return 25
            if mode in {"unknown-json", "blocked-unknown-json"}:
                text = json.dumps({
                    "action": CANARIES[1], "schema_version": CANARIES[2],
                    "unrecognized": CANARIES,
                })
            elif mode == "unknown-text":
                text = "Unrecognized analyzer response: " + " ".join(CANARIES)
            elif mode == "pending":
                text = json.dumps({"status": "not_ready", "detail": " ".join(CANARIES)})
            elif mode == "remote-error":
                text = " ".join(CANARIES)
            elif mode == "empty":
                text = "No results found."
            elif mode == "empty-ready":
                text = "No results found.\nIndexing lexical: ready\nIndexing semantic: disabled"
            else:
                raise AssertionError(mode)
            response["result"] = {"content": [{"type": "text", "text": text}]}
            if mode == "remote-error":
                response["result"]["isError"] = True
        else:
            response["error"] = {"code": -32601, "message": "unsupported fixture method"}
        print(json.dumps(response), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
