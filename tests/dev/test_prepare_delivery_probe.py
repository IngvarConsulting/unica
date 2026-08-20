"""Tests for the manual engine-delivery probe preflight.

The Unica release owns only the core. Engine bytes come from the immutable
toolchain URL declared by the packaged runtime manifest, so the preflight must
validate both origins instead of inventing an engine asset in the core release.
"""

from __future__ import annotations

import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "prepare-delivery-probe.py"
SPEC = importlib.util.spec_from_file_location("prepare_delivery_probe", SCRIPT)
PROBE = importlib.util.module_from_spec(SPEC)
sys.modules["prepare_delivery_probe"] = PROBE
SPEC.loader.exec_module(PROBE)


class DeliveryProbePreflightTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.target = PROBE.host_target()
        exe = ".exe" if self.target == "win-x64" else ""
        self.engine_asset = f"bsl-analyzer-{self.target}{exe}"
        self.engine_sha = "a" * 64
        self.toolchain_tag = "bsl-analyzer-v0.2.67-build.1"
        self.manifest = self.root / "runtime-manifest.json"
        self.manifest.write_text(
            json.dumps(
                {
                    "schemaVersion": 2,
                    "pluginVersion": "9.9.9",
                    "artifacts": {
                        "bsl-analyzer": {
                            "version": "0.2.67",
                            "role": "engine",
                            "targets": {
                                self.target: {
                                    "asset": {
                                        "name": self.engine_asset,
                                        "url": (
                                            "https://github.com/IngvarConsulting/"
                                            "unica-toolchain/releases/download/"
                                            f"{self.toolchain_tag}/{self.engine_asset}"
                                        ),
                                        "mediaType": "application/octet-stream",
                                        "sha256": self.engine_sha,
                                    },
                                    "files": [],
                                }
                            },
                        }
                    },
                }
            ),
            encoding="utf-8",
        )

    def run_probe(
        self,
        *,
        core_assets: list[str] | None = None,
        toolchain_assets: list[str] | None = None,
        extra: tuple[str, ...] = (),
    ) -> tuple[int, str]:
        core_assets = core_assets or [f"unica-runtime-{self.target}.tar.gz"]
        toolchain_assets = toolchain_assets or [self.engine_asset]

        def releases(repository: str, tag: str | None):
            if repository == "IngvarConsulting/unica":
                return "v9.9.9", core_assets
            self.assertEqual(repository, "IngvarConsulting/unica-toolchain")
            self.assertEqual(tag, self.toolchain_tag)
            return self.toolchain_tag, toolchain_assets

        argv = [
            "prepare-delivery-probe.py",
            "--manifest",
            str(self.manifest),
            "--cache",
            str(self.root / "cache"),
            *extra,
        ]
        captured = io.StringIO()
        with mock.patch.object(PROBE, "release_assets", side_effect=releases):
            with mock.patch.object(sys, "argv", argv):
                with redirect_stdout(captured):
                    code = PROBE.main()
        return code, captured.getvalue()

    def test_manifest_declared_toolchain_asset_hands_over_the_probe(self) -> None:
        code, output = self.run_probe()

        self.assertEqual(code, 0, output)
        self.assertIn("unica-toolchain", output)
        self.assertIn("io.unica/deliveryProgress", output)
        self.assertIn(
            f"bsl-analyzer/0.2.67--{self.engine_sha}/{self.target}",
            output,
        )

    def test_missing_declared_toolchain_asset_refuses_the_probe(self) -> None:
        code, output = self.run_probe(toolchain_assets=["another-asset"])

        self.assertEqual(code, 2, output)
        self.assertIn(self.engine_asset, output)
        self.assertIn("unica-toolchain", output)

    def test_engine_url_outside_toolchain_is_rejected(self) -> None:
        manifest = json.loads(self.manifest.read_text(encoding="utf-8"))
        manifest["artifacts"]["bsl-analyzer"]["targets"][self.target]["asset"]["url"] = (
            "https://github.com/IngvarConsulting/unica/releases/download/"
            f"v9.9.9/{self.engine_asset}"
        )
        self.manifest.write_text(json.dumps(manifest), encoding="utf-8")

        with self.assertRaisesRegex(SystemExit, "unica-toolchain"):
            self.run_probe()


if __name__ == "__main__":
    unittest.main()
