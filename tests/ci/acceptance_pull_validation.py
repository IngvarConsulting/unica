"""Explicit runtime-fault corpus: controlled runner, genuine platform attestation."""
from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
import sys
import tempfile
import time
import unittest
from pathlib import Path

from tests.ci.acceptance_profiles import REPO, host_target, select_profile

WORKSPACE = "tests/fixtures/acceptance/workspace-pull-validation"
CASES = {"unsupported-root", "wrong-version", "invalid-xml", "missing-owner", "empty-dump"}


def platform_executable():
    value = os.environ.get("UNICA_ACCEPTANCE_PLATFORM_ROOT")
    if not value:
        raise ValueError("runtime-fault requires UNICA_ACCEPTANCE_PLATFORM_ROOT with a genuine 8.3.27 installation")
    root = Path(value).resolve(strict=True)
    executable = root / ("1cv8.exe" if os.name == "nt" else "1cv8")
    if not executable.is_file():
        raise ValueError("runtime-fault platform root has no 1cv8 executable")
    # The standard binary performs the real installation/version attestation.
    return executable


def stage_producer(plugin):
    if os.name == "nt":
        raise ValueError("the controlled Python executable driver currently requires POSIX")
    lock = json.loads((REPO / "plugins/unica/third-party/tools.lock.json").read_text())
    target = host_target(lock, platform.system(), platform.machine())
    source = REPO / "tests/fixtures/runtime/pull-validation/producer.py"
    binary = plugin / "bin" / target / "v8-runner"
    binary.parent.mkdir(parents=True)
    binary.write_text(f"#!{sys.executable}\n" + source.read_text())
    binary.chmod(0o700)
    shutil.copyfile(REPO / WORKSPACE / "src/Configuration.xml", binary.with_name("Configuration.xml"))
    relative = binary.relative_to(plugin).as_posix()
    manifest = {"schemaVersion": 2, "targetTriple": lock["targets"][target]["targetTriple"], "tools": [{
        "name": "v8-runner", "version": "0.15.2", "binaryPath": relative, "deliveredPath": relative,
        "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    }], "testFixture": {"kind": "controlled-pull-validation", "publishedArtifact": False,
        "source": source.relative_to(REPO).as_posix(), "sourceSha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "protocolCompatibleWith": "v8-runner 0.15.2"}}
    (plugin / "third-party").mkdir()
    (plugin / "third-party/manifest.json").write_text(json.dumps(manifest))
    return manifest


def prepare_case(server, scenario):
    if scenario.get("driver") != "pull-validation-fault" or scenario.get("workspace") != WORKSPACE or scenario.get("evaluation") not in CASES:
        raise ValueError("runtime-fault requires its closed pull validation fixture")
    executable = platform_executable()
    root = server.workspace
    (root / "v8project.local.yaml").write_text(json.dumps({"tools": {"platform": {"path": str(executable.parent), "strict": True}}}))
    (root / "pull-fault.json").write_text(json.dumps({"case": scenario["evaluation"], "platform": str(executable)}))
    original_call = server.call
    frames = []

    def observed_call(tool, arguments, label=None, *, timeout=None):
        # A lost observation never authorizes a second execution. Keep the
        # failed frame as evidence and observe only the caller's same TaskId.
        deadline = time.monotonic() + (timeout if timeout is not None else 120)
        for attempt in range(3):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("runtime-fault observation deadline expired")
            response = original_call(tool, arguments, label, timeout=remaining)
            from tests.ci.acceptance_controls import tool_result
            result = tool_result(response) or {}
            frames.append({"tool": tool, "args": arguments, "result": result})
            if (tool != "unica.task.result" or not isinstance(arguments.get("taskId"), str)
                    or result.get("ok") is not False
                    or not any(d.get("code") == "task_transport_failed" for d in result.get("diagnostics", []))):
                return response
            print("runtime-fault: task transport lost; observing the same TaskId", flush=True)
        return response

    server.call = observed_call
    before = {str(p.relative_to(root / "src")): p.read_bytes() for p in (root / "src").rglob("*") if p.is_file()}

    def finish(records):
        after = {str(p.relative_to(root / "src")): p.read_bytes() for p in (root / "src").rglob("*") if p.is_file()}
        if after != before:
            raise AssertionError("refused pull changed the original target")
        final = records[-1]["result"]
        apply_calls = [frame for frame in frames if frame["tool"] == "unica.run" and frame["args"].get("dryRun") is False]
        if len(apply_calls) != 1:
            raise AssertionError("runtime-fault must dispatch exactly one apply invocation")
        encoded = json.dumps(frames)
        if any(marker in encoded for marker in ("private-runtime-secret", "runtime-absolute-marker", "raw-runtime-stderr", str(root), str(executable.parent))):
            raise AssertionError("private controlled provider input leaked")
        if not any(w.get("reason") == "writerCompletionUnconfirmed" for w in final.get("warnings", [])):
            raise AssertionError("validation failure lost its separate cleanup cause")
        artifacts = final.get("artifacts", [])
        if len(artifacts) != 1 or artifacts[0].get("kind") != "recovery" or not (root / artifacts[0]["path"]).is_dir():
            raise AssertionError("cleanup did not retain its recovery")
    return finish


def main():
    from tests.ci import test_acceptance_scenarios as harness
    platform_executable()  # Absence is an explicit error, never a host skip.
    corpus = select_profile(harness.load_acceptance_corpus(), "runtime-fault")
    harness.ensure_binary(REPO)
    with tempfile.TemporaryDirectory(prefix="unica-pull-fault-") as raw:
        plugin = Path(raw).resolve() / "plugin"
        stage_producer(plugin)
        harness.run_corpus(unittest.TestCase(), corpus, plugin, scenario_driver=prepare_case)
    print(f"PASS {len(corpus['scenarios'])} runtime-fault scenarios")


if __name__ == "__main__":
    main()
