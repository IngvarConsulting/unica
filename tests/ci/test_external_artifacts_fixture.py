from __future__ import annotations

import hashlib
import json
import subprocess
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURE = REPO_ROOT / "tests/fixtures/platform_8_3_27/external_artifacts"


def fixture_files() -> dict[str, bytes]:
    return {
        path.relative_to(FIXTURE).as_posix(): path.read_bytes()
        for path in sorted(FIXTURE.rglob("*"))
        if path.is_file()
        and path.parent != FIXTURE
        and not path.name.startswith(".")
    }


class ExternalArtifactsFixtureTests(unittest.TestCase):
    def test_manifest_lists_every_dump_file_with_its_bytes(self) -> None:
        manifest = json.loads((FIXTURE / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["platformVersion"], "8.3.27.2074")
        self.assertEqual(manifest["formatVersion"], "2.20")
        files = fixture_files()
        self.assertEqual(sorted(manifest["files"]), sorted(files))
        for name, data in files.items():
            with self.subTest(name=name):
                entry = manifest["files"][name]
                self.assertEqual(entry["size"], len(data))
                self.assertEqual(entry["sha256"], hashlib.sha256(data).hexdigest())

    def test_dump_bytes_stay_outside_checkout_normalization(self) -> None:
        paths = [str(FIXTURE / name) for name in fixture_files()]
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
