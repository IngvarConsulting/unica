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
                (bundle / "tools.json").write_text(
                    json.dumps(
                        {
                            "target": target,
                            "targetTriple": triple,
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
