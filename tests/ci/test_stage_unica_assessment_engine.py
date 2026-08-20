from __future__ import annotations

import hashlib
import json
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "stage-unica-assessment-engine.py"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def runtime_file(bundle: Path, path: Path, *, artifact: str, executable: bool) -> dict:
    return {
        "path": path.relative_to(bundle).as_posix(),
        "deliveredPath": path.name,
        "sha256": sha256(path),
        "size": path.stat().st_size,
        "executable": executable,
        "artifact": artifact,
    }


def make_bundle(root: Path) -> Path:
    bundle = root / "bundle"
    target = bundle / "bin" / "linux-x64"
    library = target / "lib" / "libpython3.12.so.1.0"
    library.parent.mkdir(parents=True)

    files = {
        "unica": (target / "unica", "unica", True),
        "bsl-analyzer": (target / "bsl-analyzer", "bsl-analyzer", True),
        "rlm-bsl-index": (target / "rlm-bsl-index", "rlm-tools-bsl", True),
        "rlm-bsl-mcp": (target / "rlm-bsl-mcp", "rlm-tools-bsl", True),
        "rlm-library": (library, "rlm-tools-bsl", False),
        "v8-runner": (target / "v8-runner", "v8-runner", True),
    }
    for name, (path, _artifact, executable) in files.items():
        path.write_bytes(name.encode("utf-8"))
        path.chmod(0o755 if executable else 0o644)

    manifest = {
        "schemaVersion": 2,
        "target": "linux-x64",
        "targetTriple": "x86_64-unknown-linux-gnu",
        "runtimeFiles": [
            runtime_file(bundle, path, artifact=artifact, executable=executable)
            for path, artifact, executable in files.values()
        ]
        + [
            {
                "deliveredPath": "manifest.json",
                "sha256": "a" * 64,
                "size": 1,
                "executable": False,
                "artifact": "rlm-tools-bsl",
            }
        ],
    }
    (bundle / "tools.json").write_text(json.dumps(manifest), encoding="utf-8")
    return bundle


class StageUnicaAssessmentEngineTests(unittest.TestCase):
    def test_packages_selected_artifacts_with_executable_modes_inside_the_archive(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root)
            archive = root / "unica-assessment-engine-linux-x64.tar.gz"

            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--bundle-root",
                    str(bundle),
                    "--out-archive",
                    str(archive),
                    "--artifact",
                    "bsl-analyzer",
                    "--artifact",
                    "rlm-tools-bsl",
                ],
                check=True,
                capture_output=True,
                text=True,
            )

            with tarfile.open(archive, "r:gz") as packaged:
                members = {member.name: member for member in packaged.getmembers()}
            self.assertEqual(
                sorted(members),
                [
                    "bin/linux-x64/bsl-analyzer",
                    "bin/linux-x64/lib/libpython3.12.so.1.0",
                    "bin/linux-x64/rlm-bsl-index",
                    "bin/linux-x64/rlm-bsl-mcp",
                ],
            )
            self.assertEqual(members["bin/linux-x64/rlm-bsl-mcp"].mode, 0o755)
            self.assertEqual(
                members["bin/linux-x64/lib/libpython3.12.so.1.0"].mode,
                0o644,
            )

    def test_stages_complete_selected_artifacts_and_excludes_other_tools(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root)
            out_dir = root / "assessment-engine"

            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--bundle-root",
                    str(bundle),
                    "--out-dir",
                    str(out_dir),
                    "--artifact",
                    "bsl-analyzer",
                    "--artifact",
                    "rlm-tools-bsl",
                ],
                check=True,
                capture_output=True,
                text=True,
            )

            self.assertEqual(
                sorted(
                    path.relative_to(out_dir).as_posix()
                    for path in out_dir.rglob("*")
                    if path.is_file()
                ),
                [
                    "bin/linux-x64/bsl-analyzer",
                    "bin/linux-x64/lib/libpython3.12.so.1.0",
                    "bin/linux-x64/rlm-bsl-index",
                    "bin/linux-x64/rlm-bsl-mcp",
                ],
            )
            self.assertEqual(
                stat.S_IMODE((out_dir / "bin/linux-x64/rlm-bsl-mcp").stat().st_mode),
                0o755,
            )
            self.assertEqual(
                stat.S_IMODE(
                    (out_dir / "bin/linux-x64/lib/libpython3.12.so.1.0").stat().st_mode
                ),
                0o644,
            )

    def test_rejects_a_tampered_selected_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root)
            (bundle / "bin/linux-x64/rlm-bsl-mcp").write_bytes(b"tampered")

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--bundle-root",
                    str(bundle),
                    "--out-dir",
                    str(root / "assessment-engine"),
                    "--artifact",
                    "rlm-tools-bsl",
                ],
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("size mismatch", result.stderr)


if __name__ == "__main__":
    unittest.main()
