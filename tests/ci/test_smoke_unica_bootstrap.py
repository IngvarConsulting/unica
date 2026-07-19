from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


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
    def fixture(self, root: Path, *, expected_sha: str | None = None) -> tuple[Path, Path]:
        plugin = root / "source-plugin"
        bootstrap = plugin / "bootstrap" / "bin" / "linux-x64" / "unica-bootstrap"
        bootstrap.parent.mkdir(parents=True)
        bootstrap.write_bytes(b"bootstrap")
        archive = root / "unica-runtime-linux-x64.tar.gz"
        archive.write_bytes(b"runtime archive")
        digest = expected_sha or hashlib.sha256(archive.read_bytes()).hexdigest()
        (plugin / "runtime-manifest.json").write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "pluginVersion": "0.7.2",
                    "development": False,
                    "source": {"repository": "source", "commit": "commit"},
                    "release": {"repository": "source", "tag": "v0.7.2"},
                    "targets": {
                        "linux-x64": {
                            "asset": {
                                "name": archive.name,
                                "url": "https://example.invalid/runtime.tar.gz",
                                "mediaType": "application/gzip",
                                "sha256": digest,
                            },
                            "entrypoint": "bin/linux-x64/unica",
                            "files": [],
                        }
                    },
                }
            ),
            encoding="utf-8",
        )
        return plugin, archive

    def test_prepare_plugin_pins_local_asset_without_changing_identity(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plugin, archive = self.fixture(root)

            prepared, bootstrap = module.prepare_plugin(
                plugin,
                archive,
                "linux-x64",
                root / "prepared",
                "http://127.0.0.1:1234/runtime.tar.gz",
            )

            manifest = json.loads(
                (prepared / "runtime-manifest.json").read_text(encoding="utf-8")
            )
            self.assertEqual(manifest["release"]["tag"], "v0.7.2")
            self.assertEqual(
                manifest["targets"]["linux-x64"]["asset"]["url"],
                "http://127.0.0.1:1234/runtime.tar.gz",
            )
            self.assertEqual(
                bootstrap,
                prepared / "bootstrap" / "bin" / "linux-x64" / "unica-bootstrap",
            )

    def test_prepare_plugin_rejects_archive_checksum_mismatch(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plugin, archive = self.fixture(root, expected_sha="0" * 64)

            with self.assertRaisesRegex(SystemExit, "sha256"):
                module.prepare_plugin(
                    plugin,
                    archive,
                    "linux-x64",
                    root / "prepared",
                    "http://127.0.0.1:1234/runtime.tar.gz",
                )


if __name__ == "__main__":
    unittest.main()
