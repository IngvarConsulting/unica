"""Synthetic managed analyzer package shared by search diagnosis checks."""
from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
from pathlib import Path

from tests.ci.test_unica_mcp_script_parity import prepare_reader_standins

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "tests/fixtures/search_failure_diagnosis/bsl_mcp.py"
CANARIES = [
    "/private/ClientCase2470/source/Module.bsl", "CompanyPrivate2470",
    "TokenPrivate2470_a9e7", "QueryPrivate2470", "StderrPrivate2470",
]


def prepare_fixture(root, mode):
    plugin, extra, log = prepare_reader_standins(root)
    target = "darwin-arm64" if platform.system() == "Darwin" else "linux-x64"
    executable = plugin / "bin" / target / "bsl-analyzer"
    shutil.copyfile(FIXTURE, executable)
    executable.chmod(0o755)
    digest = hashlib.sha256(executable.read_bytes()).hexdigest()
    manifest_path = plugin / "third-party/manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    for tool in manifest["tools"]:
        if tool["name"] == "bsl-analyzer":
            tool["binaries"][target]["sha256"] = digest
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
    lock_path = plugin / "third-party/tools.lock.json"
    lock = json.loads(lock_path.read_text(encoding="utf-8"))
    for tool in lock["tools"]:
        if tool["name"] == "bsl-analyzer":
            tool["assets"][target]["sha256"] = digest
    lock_path.write_text(json.dumps(lock), encoding="utf-8")
    workspace = root / "workspace"
    shutil.copytree(ROOT / "tests/fixtures/acceptance/workspace-git-search", workspace)
    if mode == "timeout":
        (workspace / "unica.toml").write_text(
            "[operational.code_intelligence]\nsearch_total_timeout_seconds = 1\n",
            encoding="utf-8",
        )
    extra["UNICA_SEARCH_FAILURE_FIXTURE_MODE"] = mode
    if mode == "blocked-unknown-json":
        release = root / "release-analyzer-response.fifo"
        os.mkfifo(release)
        extra["UNICA_SEARCH_FAILURE_RELEASE"] = str(release)
    return workspace, extra, log
