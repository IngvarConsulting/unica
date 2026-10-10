"""Controlled native JSONL producer; never represented as a published analyzer."""
from __future__ import annotations

import hashlib
import json
import os
import platform
import subprocess
from pathlib import Path

from tests.ci.acceptance_profiles import REPO, FAULT_CASES, host_target

FIXTURE_VERSION = "0.0.0-test-fixture"
PRIVATE_MARKERS = ("private-severity-token-1064", "/private/fault-source-1064")


def stage_fault_producer(plugin: Path) -> dict:
    lock = json.loads((REPO / "plugins/unica/third-party/tools.lock.json").read_text())
    target = host_target(lock, platform.system(), platform.machine())
    source = REPO / "tests/fixtures/analyzer/jsonl-fault-producer.rs"
    binary = plugin / "bin" / target / ("bsl-analyzer.exe" if os.name == "nt" else "bsl-analyzer")
    binary.parent.mkdir(parents=True)
    subprocess.run(["rustc", "--edition=2021", str(source), "-o", str(binary)],
                   check=True, capture_output=True, timeout=90)
    relative = binary.relative_to(plugin).as_posix()
    entry = {"name": "bsl-analyzer", "version": FIXTURE_VERSION,
             "binaryPath": relative, "deliveredPath": relative,
             "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    manifest = {"schemaVersion": 2, "targetTriple": lock["targets"][target]["targetTriple"],
                "tools": [entry], "testFixture": {
                    "source": source.relative_to(REPO).as_posix(),
                    "sourceSha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                    "kind": "controlled-jsonl-fault", "publishedArtifact": False}}
    (plugin / "third-party").mkdir()
    (plugin / "third-party/manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    return manifest


def write_fault_case(plugin: Path, case: str) -> None:
    if case not in FAULT_CASES:
        raise ValueError(f"unknown controlled JSONL case {case!r}")
    (plugin / "jsonl-fault-case").write_text(case, encoding="utf-8")


def fault_driver(plugin: Path, records: list):
    def prepare(server, scenario):
        if scenario.get("driver") != "analyzer-jsonl-fault":
            raise ValueError("fault profile requires its closed driver")
        write_fault_case(plugin, scenario["evaluation"])
        if not hasattr(server, "_fault_original_call"):
            server._fault_original_call = server.call

            def observed_call(tool, arguments, label=None, *, timeout=None):
                response = server._fault_original_call(tool, arguments, label, timeout=timeout)
                records.append({"tool": tool, "args": arguments, "response": response})
                encoded = json.dumps(response, ensure_ascii=False)
                if any(marker in encoded for marker in PRIVATE_MARKERS):
                    raise ValueError("controlled provider input leaked into the public response")
                return response

            server.call = observed_call
        return None
    return prepare
