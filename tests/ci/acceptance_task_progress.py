"""A silent local runner fixture for observing Task progress over real MCP."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import platform
import subprocess
import time
from pathlib import Path

from tests.ci.acceptance_profiles import REPO, host_target
from tests.ci.acceptance_controls import tool_result

WORKSPACE = "tests/fixtures/acceptance/workspace-task-progress"
DRIVER = "silent-extension-runner"


def prepare_silent_runner(server, scenario, *, records=None):
    if scenario.get("driver") != DRIVER:
        raise ValueError("Task progress scenario requires its controlled runner")
    lock = json.loads((REPO / "plugins/unica/third-party/tools.lock.json").read_text())
    target = host_target(lock, platform.system(), platform.machine())
    version = next(tool["version"] for tool in lock["tools"] if tool["name"] == "v8-runner")
    plugin = server.workspace / "plugins/unica"
    relative = f"bin/{target}/v8-runner" + (".exe" if os.name == "nt" else "")
    binary = plugin / relative
    binary.parent.mkdir(parents=True)
    source = REPO / "tests/fixtures/acceptance/providers/silent-extension-runner.rs"
    subprocess.run(["rustc", "--edition=2021", str(source), "-o", str(binary)],
                   check=True, capture_output=True, timeout=90)
    (plugin / "skills").mkdir()
    (plugin / "third-party").mkdir()
    manifest = {"sourceManifest": True, "tools": [{"name": "v8-runner", "version": version,
                "binaries": {target: {"binaryPath": relative,
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}}}]}
    (plugin / "third-party/manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    original_call = server.call
    original_close = server.close
    task_ids = set()
    closed = False
    record = {"pids": set(), "remaining": set(), "cancel_errors": []}
    if records is not None:
        records.append(record)


    def observed_call(tool, arguments, label=None, *, timeout=None):
        if tool == "unica.task.get":
            deadline = time.monotonic() + 20
            while not (server.workspace / "tree.pids").exists():
                if time.monotonic() >= deadline:
                    raise TimeoutError("silent runner did not reach its process wait")
                time.sleep(0.01)
        response = original_call(tool, arguments, label, timeout=timeout)
        task = ((tool_result(response) or {}).get("data") or {}).get("task") or {}
        if task.get("taskId"):
            task_ids.add(task["taskId"])
        encoded = json.dumps(response, ensure_ascii=False)
        if "never-publish-this-secret" in encoded or str(server.workspace) in encoded:
            raise ValueError("Task progress leaked private runner input")
        return response

    def close_fixture():
        nonlocal closed
        if closed:
            return
        closed = True
        try:
            try:
                for task_id in task_ids:
                    try:
                        original_call("unica.task.cancel", {"taskId": task_id}, timeout=10)
                        original_call("unica.task.result", {"taskId": task_id, "waitMs": 1000}, timeout=10)
                    except Exception as error:
                        record["cancel_errors"].append(str(error))
            finally:
                # Always release both fixture loops, including after a broken RPC.
                (server.workspace / "release.marker").touch()
                pid_file = server.workspace / "tree.pids"
                if pid_file.exists():
                    record["pids"] = {int(pid) for pid in pid_file.read_text().split()}
                    spec = importlib.util.spec_from_file_location(
                        "task_progress_process_observer", REPO / "scripts/ci/probe-unica-wire.py")
                    probe = importlib.util.module_from_spec(spec)
                    spec.loader.exec_module(probe)
                    probe._wait_for_process_pids(record["pids"], 10)
                    record["remaining"] = probe._running_pids(record["pids"])
                    if record["remaining"]:
                        raise RuntimeError(f"silent fixture processes survived cleanup: {record['remaining']}")
        finally:
            original_close()

    server.call = observed_call
    server.close = close_fixture
    return None
