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


def analyzer_lock_and_target() -> tuple[dict, str]:
    lock = json.loads((REPO / "plugins/unica/third-party/tools.lock.json").read_text())
    return lock, host_target(lock, platform.system(), platform.machine())


def stage_fault_producer(plugin: Path) -> dict:
    lock, target = analyzer_lock_and_target()
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


def place_checksum(plugin: Path, case: str, original: bytes) -> None:
    """The checksum case publishes a manifest whose digest the producer does not match.

    The scenario (S398) relies on the analysis adapter verifying the digest
    before it starts the producer: `check` itself does not hash the engine
    before the run, it names the `checksum` stage after the adapter refused.
    """
    manifest_path = plugin / "third-party/manifest.json"
    if case != "checksum-mismatch":
        manifest_path.write_bytes(original)
        return
    manifest = json.loads(original)
    manifest["tools"][0]["sha256"] = "0" * 64
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")


def analyzer_binary(plugin: Path) -> Path:
    return plugin / "bin" / analyzer_lock_and_target()[1] / ("bsl-analyzer.exe" if os.name == "nt" else "bsl-analyzer")


def place_engine(plugin: Path, present: bool) -> None:
    """The missing-engine case removes the producer; every other case restores it."""
    binary = analyzer_binary(plugin)
    aside = binary.with_name(binary.name + ".absent")
    if present and aside.exists():
        aside.rename(binary)
    elif not present and binary.exists():
        binary.rename(aside)


def write_fault_case(plugin: Path, case: str) -> None:
    if case not in FAULT_CASES:
        raise ValueError(f"unknown controlled JSONL case {case!r}")
    (plugin / "jsonl-fault-case").write_text(case, encoding="utf-8")


def fault_driver(plugin: Path, records: list):
    original = (plugin / "third-party/manifest.json").read_bytes()

    def prepare(server, scenario):
        if scenario.get("driver") != "analyzer-jsonl-fault":
            raise ValueError("fault profile requires its closed driver")
        write_fault_case(plugin, scenario["evaluation"])
        place_engine(plugin, scenario["evaluation"] != "missing-engine")
        place_checksum(plugin, scenario["evaluation"], original)
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


def stage_missing_analyzer(plugin: Path) -> dict:
    """A package that names the analyzer but has neither its binary nor a
    runtime manifest: ordering it fails as a configuration delivery failure."""
    lock = json.loads((REPO / "plugins/unica/third-party/tools.lock.json").read_text())
    target = host_target(lock, platform.system(), platform.machine())
    relative = f"bin/{target}/" + ("bsl-analyzer.exe" if os.name == "nt" else "bsl-analyzer")
    entry = {"name": "bsl-analyzer", "version": FIXTURE_VERSION,
             "binaryPath": relative, "deliveredPath": relative, "sha256": "0" * 64}
    manifest = {"schemaVersion": 2, "targetTriple": lock["targets"][target]["targetTriple"],
                "tools": [entry], "testFixture": {
                    "kind": "missing-analyzer-delivery-failure", "publishedArtifact": False}}
    (plugin / "third-party").mkdir(parents=True)
    (plugin / "third-party/manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    if (plugin / relative).exists() or (plugin / "runtime-manifest.json").exists():
        raise ValueError("the delivery-failure package must not carry the analyzer or a manifest")
    return manifest


def delivery_failure_driver(plugin: Path):
    def prepare(server, scenario):
        if scenario.get("driver") != "analyzer-delivery-failure":
            raise ValueError("delivery-failure run requires its closed driver")
        return None
    return prepare
