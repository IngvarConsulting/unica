from __future__ import annotations

import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "smoke-unica-bootstrap.py"


def load_module():
    spec = importlib.util.spec_from_file_location("smoke_unica_bootstrap", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SmokeUnicaBootstrapTests(unittest.TestCase):
    def plugin(self, root: Path) -> Path:
        plugin = root / "plugin"
        bootstrap = plugin / "bootstrap" / "bin" / "linux-x64" / "unica-bootstrap"
        bootstrap.parent.mkdir(parents=True)
        bootstrap.write_bytes(b"bootstrap")
        return plugin

    def test_probe_accepts_controlled_download_failure(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            result = subprocess.CompletedProcess(
                args=[],
                returncode=1,
                stdout="",
                stderr="unica-bootstrap: failed to download runtime: HTTP 404",
            )

            with patch.object(module.subprocess, "run", return_value=result):
                module.smoke(
                    plugin,
                    "linux-x64",
                    2,
                    expect_download_failure=True,
                )

    def test_probe_rejects_stack_overflow_before_download_error(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            result = subprocess.CompletedProcess(
                args=[],
                returncode=1,
                stdout="",
                stderr="thread 'main' has overflowed its stack",
            )

            with patch.object(module.subprocess, "run", return_value=result):
                with self.assertRaisesRegex(SystemExit, "overflowed its stack"):
                    module.smoke(
                        plugin,
                        "linux-x64",
                        2,
                        expect_download_failure=True,
                    )

    def test_release_smoke_requires_success_marker(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            result = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout="",
                stderr="verified Unica runtime 0.7.3 and MCP tools",
            )

            with patch.object(module.subprocess, "run", return_value=result):
                module.smoke(
                    plugin,
                    "linux-x64",
                    2,
                    expect_download_failure=False,
                )


if __name__ == "__main__":
    unittest.main()
