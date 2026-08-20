from __future__ import annotations

import importlib.util
import json
import shutil
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
                        "artifactAssets": {},
                        "runtimeFiles": [
                            {
                                "path": "bin/linux-x64/unica",
                                "deliveredPath": "bin/linux-x64/unica",
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

    def test_the_release_refuses_to_carry_an_engine_archive(self) -> None:
        """Движки издаёт тулчейн, и выпуск их не перепубликует.

        Лишний архив рядом с ядром означает, что 439 МБ чужих байтов снова
        поехали в выпуск: проверка обязана остановить это раньше публикации.
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
                        "artifactAssets": {
                            "bsl-analyzer": {
                                "repository": "https://github.com/IngvarConsulting/unica-toolchain",
                                "tag": "bsl-analyzer-v0.2.67-build.1",
                                "name": "bsl-analyzer-linux-x64",
                                "mediaType": "application/octet-stream",
                                "sha256": "c" * 64,
                            }
                        },
                        "runtimeFiles": [
                            {
                                "path": "bin/linux-x64/unica",
                                "deliveredPath": "bin/linux-x64/unica",
                                "sha256": packager.sha256(core),
                                "size": core.stat().st_size,
                                "executable": True,
                                "artifact": "unica",
                            },
                            {
                                "path": "bin/linux-x64/bsl-analyzer",
                                "deliveredPath": "bsl-analyzer",
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

            # Движок описан адресом, а архивом выходит только ядро.
            self.assertEqual(verifier.published_artifacts(assets, "linux-x64"), ["unica"])
            self.assertTrue((assets / "bsl-analyzer-runtime-linux-x64.json").is_file())
            self.assertFalse((assets / "bsl-analyzer-runtime-linux-x64.tar.gz").exists())

            shutil.copy2(
                assets / "unica-runtime-linux-x64.tar.gz",
                assets / "bsl-analyzer-runtime-linux-x64.tar.gz",
            )
            with self.assertRaisesRegex(SystemExit, "must be \\[unica\\]"):
                verifier.published_artifacts(assets, "linux-x64")

    def test_one_target_is_checked_for_composition_too(self) -> None:
        """Состав проверяется там, где он создаётся, а не после публикации.

        Сборка зовёт проверяльщик поцелево, и он смотрел только пару ядра:
        лишний архив дожил бы до выкладки и вскрылся бы шагом позже, уже
        опубликованным.
        """
        verifier = load_module(REPO_ROOT / "scripts/ci/verify-release-assets.py", "asset_verifier")
        packager = load_module(REPO_ROOT / "scripts/ci/package-unica-runtime.py", "runtime_packager")

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
                        "artifactAssets": {},
                        "runtimeFiles": [
                            {
                                "path": "bin/linux-x64/unica",
                                "deliveredPath": "bin/linux-x64/unica",
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

            self.assertEqual(
                verifier.verify_release_target(assets, "linux-x64"), "0.7.0"
            )

            shutil.copy2(
                assets / "unica-runtime-linux-x64.tar.gz",
                assets / "rlm-tools-bsl-runtime-linux-x64.tar.gz",
            )
            with self.assertRaisesRegex(SystemExit, "must be \\[unica\\]"):
                verifier.verify_release_target(assets, "linux-x64")

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
                            "artifactAssets": {},
                            "runtimeFiles": [
                                {
                                    "path": f"bin/{target}/unica{exe}",
                                    "deliveredPath": f"bin/{target}/unica{exe}",
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
