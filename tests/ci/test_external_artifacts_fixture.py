from __future__ import annotations

import hashlib
import json
import subprocess
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURES = {
    "2.20": "8.3.27.2074",
    "2.21": "8.5.1.1529",
    "2.22": "8.5.4.2028",
}


def fixture_root(format_version: str) -> Path:
    return REPO_ROOT / f"tests/fixtures/formats/platform_xml/{format_version}/external_artifacts"


def fixture_files(root: Path) -> dict[str, bytes]:
    return {
        path.relative_to(root).as_posix(): path.read_bytes()
        for path in sorted(root.rglob("*"))
        if path.is_file()
        and path.parent != root
        and not path.name.startswith(".")
    }


class ExternalArtifactsFixtureTests(unittest.TestCase):
    def test_manifest_lists_every_dump_file_with_its_bytes(self) -> None:
        for format_version, platform_version in FIXTURES.items():
            with self.subTest(format_version=format_version):
                root = fixture_root(format_version)
                manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
                self.assertEqual(manifest["platformVersion"], platform_version)
                self.assertEqual(manifest["formatVersion"], format_version)
                files = fixture_files(root)
                self.assertEqual(sorted(manifest["files"]), sorted(files))
                for name, data in files.items():
                    with self.subTest(name=name):
                        entry = manifest["files"][name]
                        self.assertEqual(entry["size"], len(data))
                        self.assertEqual(entry["sha256"], hashlib.sha256(data).hexdigest())

    def test_dump_bytes_stay_outside_checkout_normalization(self) -> None:
        for format_version in FIXTURES:
            with self.subTest(format_version=format_version):
                root = fixture_root(format_version)
                paths = [str(root / name) for name in fixture_files(root)]
                output = subprocess.run(
                    ["git", "check-attr", "text", "--", *paths],
                    cwd=REPO_ROOT,
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout
                lines = output.splitlines()
                self.assertEqual(len(lines), len(paths))
                for line in lines:
                    self.assertTrue(line.endswith(": text: unset"), line)


if __name__ == "__main__":
    unittest.main()
