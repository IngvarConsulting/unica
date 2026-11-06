"""Source-corpus supplier seam; never changes the product's engine routing."""
from __future__ import annotations

import hashlib
import json
import os
import platform
from pathlib import Path
import shutil
import subprocess
import sys

from tests.ci.acceptance_profiles import REPO, host_target, isolated_environment

WORKSPACE = "tests/fixtures/acceptance/workspace-support-restore"
NATIVE = REPO / "tests/fixtures/platform_8_3_27/support-edit-bin-only/src"
PRODUCER = REPO / "tests/fixtures/runtime/support-restore/producer.py"
LAUNCHER = REPO / "tests/fixtures/runtime/support-restore/launcher.rs"


def prepare_workspace(workspace: Path, state: Path):
    shutil.copytree(NATIVE, workspace / "src")
    plugin = state / "controlled-supplier-plugin"
    lock_path = REPO / "plugins/unica/third-party/tools.lock.json"
    lock = json.loads(lock_path.read_text())
    target = host_target(lock, platform.system(), platform.machine())
    directory = plugin / "bin" / target
    directory.mkdir(parents=True)
    binary = directory / ("v8-runner.exe" if os.name == "nt" else "v8-runner")
    subprocess.run(["rustc", "--edition=2021", str(LAUNCHER), "-o", str(binary)],
                   check=True, capture_output=True, timeout=90)
    shutil.copyfile(PRODUCER, directory / "producer.py")
    (directory / "python-path").write_text(sys.executable, encoding="utf-8")
    # This snapshot is independent of the product's mutable source tree.
    shutil.copytree(NATIVE, directory / "baseline")
    relative = binary.relative_to(plugin).as_posix()
    manifest = {"schemaVersion": 2, "targetTriple": lock["targets"][target]["targetTriple"],
                "tools": [{"name": "v8-runner", "version": "0.15.3",
                           "binaryPath": relative, "deliveredPath": relative,
                           "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}],
                "testFixture": {"kind": "controlled-supplier-conversion", "publishedArtifact": False,
                                "protocolCompatibleWith": "v8-runner 0.15.3",
                                "source": PRODUCER.relative_to(REPO).as_posix(),
                                "sourceSha256": hashlib.sha256(PRODUCER.read_bytes()).hexdigest(),
                                "launcherSource": LAUNCHER.relative_to(REPO).as_posix(),
                                "launcherSha256": hashlib.sha256(LAUNCHER.read_bytes()).hexdigest()}}
    (plugin / "third-party").mkdir()
    shutil.copyfile(lock_path, plugin / "third-party/tools.lock.json")
    (plugin / "third-party/manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    return isolated_environment(plugin, state)
