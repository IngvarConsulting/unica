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
# Suite planning: which catalog tasks run, and why the others do not.
# --------------------------------------------------------------------------


def plan_suite(tasks: list[dict], *, corpus_available: bool) -> list[dict]:
    """Decide, for every task, whether it runs — and record why it does not.

    A task the suite cannot run is reported as skipped, never dropped: a
    silently shortened suite reads as full coverage and is how nine measured
    tasks come to be reported as twelve.
    """
    plan = []
    for task in sorted(tasks, key=lambda entry: entry["name"]):
        requirement = task.get("requires")
        if requirement and not corpus_available:
            plan.append(
                {
                    "task": task["name"],
                    "action": "skip",
                    "reason": f"{requirement} is not set; the task needs an external corpus",
                }
            )
            continue
        plan.append({"task": task["name"], "action": "run", "reason": None})
    return plan


def aggregate_suite(results: list[dict], *, tokenizer: str) -> dict:
    """Sum what was measured, and keep what was not in plain sight."""
    measured = [result for result in results if result["status"] == "measured"]
    skipped = [result for result in results if result["status"] == "skipped"]
    return {
        "tokenizer": tokenizer,
        "measured": len(measured),
        "skipped": len(skipped),
        "total_tokens": sum(result["total_tokens"] for result in measured),
        "tasks": results,
    }


FIXTURE_ROOT = REPO_ROOT / "tests" / "fixtures"
WORKSPACE_SEEDS = {
    "minimal": FIXTURE_ROOT / "unica_mcp_script_parity" / "meta-validate-language-aware",
    "xdto": FIXTURE_ROOT / "xdto" / "enterprise-data-minimal",
}
V8PROJECT = (
    "format: DESIGNER\nsource-set:\n"
    "  - name: main\n    type: CONFIGURATION\n    path: src\n"
)


def materialize_workspace(kind: str, root: Path, *, corpus: Path | None) -> Path:
    """Prepare the workspace an episode runs in.

    Seeded kinds are copied into a throwaway directory, so a mutating episode
    measures the same starting state on every run and never touches the
    repository tree. `corpus` points at an external checkout and is used by
    read-only episodes only.
    """
    if kind == "none":
        return root
    if kind == "corpus":
        if corpus is None:
            raise SystemExit("workspace 'corpus' requires --corpus or UNICA_TOKEN_COST_CORPUS")
        return corpus
    seed = WORKSPACE_SEEDS.get(kind)
    if seed is None:
        raise SystemExit(f"unknown workspace {kind!r}; expected none, corpus, or one of "
                         f"{', '.join(sorted(WORKSPACE_SEEDS))}")
    source = root / "src"
    source.mkdir(parents=True, exist_ok=True)
    for path in sorted(seed.rglob("*")):
        if path.is_file():
            target = source / path.relative_to(seed)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(path.read_bytes())
    (root / "v8project.yaml").write_text(V8PROJECT, encoding="utf-8")
    return root


def load_tasks(directory: Path) -> list[dict]:
    """Read every task file, naming each after its file stem."""
    tasks = []
    for path in sorted(directory.glob("*.json")):
        task = json.loads(path.read_text(encoding="utf-8"))
        task["name"] = path.stem
        tasks.append(task)
    return tasks


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


def run_suite(
    *, binary: Path, directory: Path, tokenizer: str, corpus: Path | None
) -> dict:
    """Run every task in `directory`, reporting the skipped ones by name."""
    import tempfile

    tasks = {task["name"]: task for task in load_tasks(directory)}
    plan = plan_suite(list(tasks.values()), corpus_available=corpus is not None)
    results = []
    for entry in plan:
        task = tasks[entry["task"]]
        if entry["action"] == "skip":
            results.append(
                {"task": entry["task"], "status": "skipped", "reason": entry["reason"]}
            )
            continue
        with tempfile.TemporaryDirectory() as temporary:
            workspace = materialize_workspace(
                task.get("workspace", "none"), Path(temporary), corpus=corpus
            )
            report = run_episode(
                binary=binary,
                episode=task["steps"],
                tokenizer=tokenizer,
                cwd=workspace,
            )
        results.append(
            {
                "task": entry["task"],
                "status": "measured",
                "goal": task.get("goal", ""),
                "catalog_task": task.get("catalog_task"),
                "total_tokens": report["total_tokens"],
                "discovery_tokens": report["discovery_tokens"],
                "calls": report["calls"],
            }
        )
    return aggregate_suite(results, tokenizer=tokenizer)


def main(argv: list[str] | None = None) -> int:
    import os

    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", required=True, type=Path, help="unica MCP binary")
    parser.add_argument("--episode", type=Path, default=None, help="single episode JSON file")
    parser.add_argument("--suite", type=Path, default=None, help="directory of task files")
    parser.add_argument("--cwd", type=Path, default=None, help="workspace for --episode")
    parser.add_argument("--corpus", type=Path, default=None, help="external corpus checkout")
    parser.add_argument("--tokenizer", choices=TOKENIZERS, default="bytes")
    parser.add_argument("--report", type=Path, default=None, help="write the report here")
    arguments = parser.parse_args(argv)

    if not arguments.binary.is_file():
        raise SystemExit(f"binary not found: {arguments.binary}")
    # The session runs with the workspace as its cwd, so a relative binary path
    # would resolve against the wrong directory once the episode starts.
    arguments.binary = arguments.binary.resolve()
    if bool(arguments.episode) == bool(arguments.suite):
        raise SystemExit("pass exactly one of --episode or --suite")

    if arguments.suite:
        corpus = arguments.corpus
        if corpus is None and os.environ.get("UNICA_TOKEN_COST_CORPUS"):
            corpus = Path(os.environ["UNICA_TOKEN_COST_CORPUS"])
        report = run_suite(
            binary=arguments.binary,
            directory=arguments.suite,
            tokenizer=arguments.tokenizer,
            corpus=corpus,
        )
    else:
        episode = json.loads(arguments.episode.read_text(encoding="utf-8"))
        steps = episode["steps"] if isinstance(episode, dict) else episode
        report = run_episode(
            binary=arguments.binary,
            episode=steps,
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
