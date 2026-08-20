from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class VerifyReleaseAssetsTests(unittest.TestCase):
    def test_verifies_one_packaged_runtime_pair_and_detects_tampering(self) -> None:
        packager = load_module(REPO_ROOT / "scripts/ci/package-unica-runtime.py", "runtime_packager")
        verifier = load_module(REPO_ROOT / "scripts/ci/verify-release-assets.py", "asset_verifier")

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = root / "linux-x64"
            binary = bundle / "bin" / "linux-x64" / "unica"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"linux-x64")
            binary.chmod(0o755)
            (bundle / "tools.json").write_text(
                json.dumps(
                    {
                        "schemaVersion": 2,
                        "target": "linux-x64",
                        "targetTriple": "x86_64-unknown-linux-gnu",
                        "runtimeFiles": [
                            {
                                "path": "bin/linux-x64/unica",
                                "sha256": packager.sha256(binary),
                                "size": binary.stat().st_size,
                                "executable": True,
                                # Разрез поставки: каждый файл называет архив,
                                # в котором он едет.
                                "artifact": "unica",
                            }
                        ],
                        "tools": [
                            {
                                "name": "unica",
                                "version": "0.7.0",
                                "targetTriple": "x86_64-unknown-linux-gnu",
                                "binaryPath": "bin/linux-x64/unica",
                                "sha256": packager.sha256(binary),
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            assets = root / "assets"
            packager.package_runtime(bundle, assets)

            self.assertEqual(verifier.verify_runtime_asset_pair(assets, "linux-x64"), "0.7.0")
            with (assets / "unica-runtime-linux-x64.tar.gz").open("ab") as stream:
                stream.write(b"tampered")
            with self.assertRaisesRegex(SystemExit, "archive checksum mismatch"):
                verifier.verify_runtime_asset_pair(assets, "linux-x64")

    def test_an_engine_archive_is_verified_too(self) -> None:
        """Разрез поставки положил движки в собственные архивы.

        Проверять один сердечник значит выпустить движки, которых никто не
        сверял: подмена такого архива дошла бы до пользователя.
        """
        packager = load_module(REPO_ROOT / "scripts/ci/package-unica-runtime.py", "runtime_packager")
        verifier = load_module(REPO_ROOT / "scripts/ci/verify-release-assets.py", "asset_verifier")

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = root / "linux-x64"
            core = bundle / "bin" / "linux-x64" / "unica"
            core.parent.mkdir(parents=True)
            core.write_bytes(b"linux-x64")
            core.chmod(0o755)
            engine = bundle / "bin" / "linux-x64" / "bsl-analyzer"
            engine.write_bytes(b"analyzer")
            engine.chmod(0o755)
            (bundle / "tools.json").write_text(
                json.dumps(
                    {
                        "schemaVersion": 2,
                        "target": "linux-x64",
                        "targetTriple": "x86_64-unknown-linux-gnu",
                        "runtimeFiles": [
                            {
                                "path": "bin/linux-x64/unica",
                                "sha256": packager.sha256(core),
                                "size": core.stat().st_size,
                                "executable": True,
                                "artifact": "unica",
                            },
                            {
                                "path": "bin/linux-x64/bsl-analyzer",
                                "sha256": packager.sha256(engine),
                                "size": engine.stat().st_size,
                                "executable": True,
                                "artifact": "bsl-analyzer",
                            },
                        ],
                        "tools": [
                            {
                                "name": "unica",
                                "version": "0.7.0",
                                "targetTriple": "x86_64-unknown-linux-gnu",
                                "binaryPath": "bin/linux-x64/unica",
                                "sha256": packager.sha256(core),
                            },
                            {
                                "name": "bsl-analyzer",
                                "version": "0.2.67",
                                "targetTriple": "x86_64-unknown-linux-gnu",
                                "binaryPath": "bin/linux-x64/bsl-analyzer",
                                "sha256": packager.sha256(engine),
                            },
                        ],
                    }
                ),
                encoding="utf-8",
            )
            assets = root / "assets"
            packager.package_runtime(bundle, assets)

            self.assertEqual(
                verifier.published_artifacts(assets, "linux-x64"),
                ["bsl-analyzer", "unica"],
            )
            self.assertEqual(
                verifier.verify_runtime_asset_pair(assets, "linux-x64", "bsl-analyzer"),
                "0.7.0",
            )

            with (assets / "bsl-analyzer-runtime-linux-x64.tar.gz").open("ab") as stream:
                stream.write(b"tampered")
            with self.assertRaisesRegex(SystemExit, "archive checksum mismatch"):
                verifier.verify_runtime_asset_pair(assets, "linux-x64", "bsl-analyzer")

    def test_verifies_three_packaged_runtime_pairs_and_detects_tampering(self) -> None:
        packager = load_module(REPO_ROOT / "scripts/ci/package-unica-runtime.py", "runtime_packager")
        verifier = load_module(REPO_ROOT / "scripts/ci/verify-release-assets.py", "asset_verifier")
        triples = {
            "darwin-arm64": "aarch64-apple-darwin",
            "linux-x64": "x86_64-unknown-linux-gnu",
            "win-x64": "x86_64-pc-windows-msvc",
        }

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            assets = root / "assets"
            for target, triple in triples.items():
                bundle = root / target
                exe = ".exe" if target == "win-x64" else ""
                binary = bundle / "bin" / target / f"unica{exe}"
                binary.parent.mkdir(parents=True)
                binary.write_bytes(target.encode())
                binary.chmod(0o755)
                (bundle / "tools.json").write_text(
                    json.dumps(
                        {
                            "schemaVersion": 2,
                            "target": target,
                            "targetTriple": triple,
                            "runtimeFiles": [
                                {
                                    "path": f"bin/{target}/unica{exe}",
                                    "sha256": packager.sha256(binary),
                                    "size": binary.stat().st_size,
                                    "executable": True,
                                    "artifact": "unica",
                                }
                            ],
                            "tools": [
                                {
                                    "name": "unica",
                                    "version": "0.7.0",
                                    "targetTriple": triple,
                                    "binaryPath": f"bin/{target}/unica{exe}",
                                    "sha256": packager.sha256(binary),
                                }
                            ],
                        }
                    ),
                    encoding="utf-8",
                )
                packager.package_runtime(bundle, assets)

            self.assertEqual(verifier.verify_release_assets(assets), "0.7.0")
            with (assets / "unica-runtime-linux-x64.tar.gz").open("ab") as stream:
                stream.write(b"tampered")
            with self.assertRaisesRegex(SystemExit, "archive checksum mismatch"):
                verifier.verify_release_assets(assets)


if __name__ == "__main__":
    unittest.main()
