#!/usr/bin/env python3
"""Measure the wire-cut cost of an MCP episode in tokens (#479).

The metric this serves is «токены на решение»: the whole cost of an episode
from the task to the confirmed result, including the discovery layer and
including every retry — re-guessing is charged, not forgiven.

**What a frame is.** Each counted frame is the JSON-RPC payload canonicalized
to compact JSON before counting, so a server's outer whitespace never enters
the measurement. Strings inside a frame are preserved verbatim, which is what
keeps this honest: a pretty-printed tool result lives inside a string value,
so its indentation is still counted, exactly as the model pays for it.

**Two tokenizers.** `bytes` counts UTF-8 bytes and needs no dependency, which
is what lets this run anywhere; `o200k_base` needs `tiktoken` and is the unit
the catalog reports in. Reports name the tokenizer they used.

The report carries no durations, timestamps or machine paths: two runs of the
same binary over the same episode must produce identical bytes, so a diff of
two reports is a diff of the surface.

Usage:
    measure-token-cost.py --binary target/release/unica \\
        --episode tests/fixtures/token_cost/tasks/read-object.json \\
        [--cwd <workspace>] [--tokenizer bytes|o200k_base] [--report out.json]
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
SMOKE_SCRIPT = REPO_ROOT / "scripts" / "ci" / "smoke-unica-mcp.py"

PROTOCOL_VERSION = "2025-06-18"
TOKENIZERS = ("bytes", "o200k_base")
DEFAULT_TIMEOUT_SECONDS = 120.0


# --------------------------------------------------------------------------
# Pure core: tokenization and accounting. No transport, no clock, no paths.
# --------------------------------------------------------------------------


def count_tokens(text: str, tokenizer: str) -> int:
    """Count `text` in the requested unit."""
    if tokenizer == "bytes":
        return len(text.encode("utf-8"))
    if tokenizer == "o200k_base":
        try:
            import tiktoken
        except ImportError:
            tiktoken = None
        if tiktoken is None:
            raise SystemExit(
                "tokenizer o200k_base requires the tiktoken package; install it "
                "(pip install tiktoken) or measure with --tokenizer bytes"
            )
        return len(tiktoken.get_encoding("o200k_base").encode(text))
    raise SystemExit(f"unknown tokenizer {tokenizer!r}; expected one of {', '.join(TOKENIZERS)}")


def canonical_frame(payload: Any) -> str:
    """Serialize a frame the way the measurement defines it: compact JSON."""
    return json.dumps(payload, ensure_ascii=False, separators=(",", ":"))


def build_report(*, discovery: str, calls: list[dict], tokenizer: str) -> dict:
    """Account one episode.

    `discovery` is the canonical `tools/list` frame; each call carries its
    canonical request and result frames plus whether the result was an error.
    Errors are counted like any other result — the metric charges for retries.
    """
    discovery_tokens = count_tokens(discovery, tokenizer)
    counted = []
    for call in calls:
        request_tokens = count_tokens(call["request"], tokenizer)
        result_tokens = count_tokens(call["result"], tokenizer)
        counted.append(
            {
                "tool": call["tool"],
                "request_tokens": request_tokens,
                "result_tokens": result_tokens,
                "is_error": bool(call["is_error"]),
            }
        )
    total = discovery_tokens + sum(c["request_tokens"] + c["result_tokens"] for c in counted)
    return {
        "tokenizer": tokenizer,
        "discovery_tokens": discovery_tokens,
        "calls": counted,
        "total_tokens": total,
    }


# --------------------------------------------------------------------------
# Transport: one stdio session, borrowed rather than reimplemented.
# --------------------------------------------------------------------------


def load_mcp_session() -> Any:
    """Reuse the smoke runner's session.

    It already owns request/notify/close, the timeout budget and the process
    tree teardown that also reaps workspace hidden services. A second copy of
    that logic is a second thing to keep correct.
    """
    spec = importlib.util.spec_from_file_location("smoke_unica_mcp", SMOKE_SCRIPT)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module.McpSession


def run_episode(
    *,
    binary: Path,
    episode: list[dict],
    tokenizer: str,
    cwd: Path,
    timeout_seconds: float = DEFAULT_TIMEOUT_SECONDS,
) -> dict:
    """Run every step of `episode` against `binary` and account the frames."""
    import os

    session_class = load_mcp_session()
    session = session_class(
        [str(binary)],
        dict(os.environ),
        timeout_seconds,
        cwd=cwd,
    )
    try:
        session.request(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "measure-token-cost", "version": "0"},
                },
            }
        )
        session.notify({"jsonrpc": "2.0", "method": "notifications/initialized"})
        listing = session.request({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        discovery = canonical_frame(listing.get("result", {}))

        calls = []
        for index, step in enumerate(episode, start=3):
            params = {"name": step["tool"], "arguments": step.get("arguments", {})}
            response = session.request(
                {"jsonrpc": "2.0", "id": index, "method": "tools/call", "params": params}
            )
            result = response.get("result")
            is_error = "result" not in response or bool(result.get("isError"))
            payload = result if result is not None else response.get("error", {})
            calls.append(
                {
                    "tool": step["tool"],
                    "request": canonical_frame(params),
                    "result": canonical_frame(payload),
                    "is_error": is_error,
                }
            )
    finally:
        session.close()

    return build_report(discovery=discovery, calls=calls, tokenizer=tokenizer)


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", required=True, type=Path, help="unica MCP binary")
    parser.add_argument("--episode", required=True, type=Path, help="episode JSON file")
    parser.add_argument("--cwd", type=Path, default=None, help="workspace for the episode")
    parser.add_argument("--tokenizer", choices=TOKENIZERS, default="bytes")
    parser.add_argument("--report", type=Path, default=None, help="write the report here")
    arguments = parser.parse_args(argv)

    if not arguments.binary.is_file():
        raise SystemExit(f"binary not found: {arguments.binary}")
    episode = json.loads(arguments.episode.read_text(encoding="utf-8"))
    report = run_episode(
        binary=arguments.binary,
        episode=episode,
        tokenizer=arguments.tokenizer,
        cwd=arguments.cwd or Path.cwd(),
    )
    serialized = json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True)
    if arguments.report:
        arguments.report.write_text(serialized + "\n", encoding="utf-8")
    print(serialized)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
