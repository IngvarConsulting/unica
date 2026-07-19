from __future__ import annotations

import hashlib
import importlib.util
import json
import tarfile
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "package-unica-runtime.py"


def load_module():
    spec = importlib.util.spec_from_file_location("package_unica_runtime", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def make_bundle(root: Path) -> Path:
    bundle = root / "bundle"
    bin_dir = bundle / "bin" / "linux-x64"
    bin_dir.mkdir(parents=True)
    unica = bin_dir / "unica"
    analyzer = bin_dir / "bsl-analyzer"
    unica.write_bytes(b"unica")
    analyzer.write_bytes(b"analyzer")
    (bundle / "tools.json").write_text(
        json.dumps(
            {
                "target": "linux-x64",
                "targetTriple": "x86_64-unknown-linux-gnu",
                "tools": [
                    {
                        "name": "unica",
                        "version": "0.7.0",
                        "binaryPath": "bin/linux-x64/unica",
                        "sha256": sha256(unica),
                    },
                    {
                        "name": "bsl-analyzer",
                        "version": "0.2.55",
                        "binaryPath": "bin/linux-x64/bsl-analyzer",
                        "sha256": sha256(analyzer),
                    },
                ],
            }
        ),
        encoding="utf-8",
    )
    return bundle


class PackageUnicaRuntimeTests(unittest.TestCase):
    def test_runtime_archive_is_deterministic_and_target_only(self) -> None:
        module = load_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root)
            first = root / "first"
            second = root / "second"
            first_archive, first_metadata = module.package_runtime(bundle, first)
            second_archive, second_metadata = module.package_runtime(bundle, second)

            self.assertEqual(first_archive.read_bytes(), second_archive.read_bytes())
            self.assertEqual(first_metadata.read_bytes(), second_metadata.read_bytes())
            with tarfile.open(first_archive, "r:gz") as archive:
                members = archive.getmembers()
            self.assertEqual(
                [member.name for member in members],
                [
                    "bin/linux-x64/bsl-analyzer",
                    "bin/linux-x64/unica",
                    "third-party/manifest.json",
                ],
            )
            self.assertTrue(all(member.mtime == 0 for member in members))

    def test_metadata_hashes_archive_and_each_runtime_file(self) -> None:
        module = load_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root)
            archive, metadata_path = module.package_runtime(bundle, root / "out")
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))

            self.assertEqual(metadata["schemaVersion"], 1)
            self.assertEqual(metadata["target"], "linux-x64")
            self.assertEqual(metadata["pluginVersion"], "0.7.0")
            self.assertEqual(metadata["asset"]["name"], "unica-runtime-linux-x64.tar.gz")
            self.assertEqual(metadata["asset"]["sha256"], sha256(archive))
            self.assertEqual(metadata["entrypoint"], "bin/linux-x64/unica")
            expected = {
                "bin/linux-x64/bsl-analyzer": hashlib.sha256(b"analyzer").hexdigest(),
                "bin/linux-x64/unica": hashlib.sha256(b"unica").hexdigest(),
            }
            actual = {item["path"]: item["sha256"] for item in metadata["files"]}
            for path, digest in expected.items():
                self.assertEqual(actual[path], digest)
            self.assertIn("third-party/manifest.json", actual)

    def test_runtime_packager_rejects_symlinked_binary(self) -> None:
        module = load_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root)
            binary = bundle / "bin" / "linux-x64" / "unica"
            binary.unlink()
            binary.symlink_to(bundle / "bin" / "linux-x64" / "bsl-analyzer")

            with self.assertRaisesRegex(SystemExit, "symlink"):
                module.package_runtime(bundle, root / "out")


if __name__ == "__main__":
    unittest.main()
