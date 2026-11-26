from __future__ import annotations

import hashlib
import json
import os
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

if __package__:
    from .assessment_engine_fixture import run_windows_engine, windows_engine_bytes
else:
    from assessment_engine_fixture import run_windows_engine, windows_engine_bytes


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


def make_bundle(root: Path, *, windows: bool = False) -> Path:
    bundle = root / "bundle"
    target = bundle / "bin" / ("win-x64" if windows else "linux-x64")
    extension = ".exe" if windows else ""
    library = target / "lib" / ("python312.dll" if windows else "libpython3.12.so.1.0")
    library.parent.mkdir(parents=True)

    files = {
        "unica": (target / f"unica{extension}", "unica", True),
        "bsl-analyzer": (target / f"bsl-analyzer{extension}", "bsl-analyzer", True),
        "tool-index": (target / f"tool-index{extension}", "tools-archive", True),
        "tool-server": (target / f"tool-server{extension}", "tools-archive", True),
        "archive-library": (library, "tools-archive", False),
        "v8-runner": (target / f"v8-runner{extension}", "v8-runner", True),
    }
    for name, (path, _artifact, executable) in files.items():
        payload = windows_engine_bytes() if windows and name == "tool-server" else name.encode("utf-8")
        path.write_bytes(payload)
        path.chmod(0o755 if executable else 0o644)

    manifest = {
        "schemaVersion": 2,
        "target": target.name,
        "targetTriple": "x86_64-pc-windows-msvc" if windows else "x86_64-unknown-linux-gnu",
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
                "artifact": "tools-archive",
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
                    "tools-archive",
                ],
                check=True,
                capture_output=True,
                text=True,
            )

            with tarfile.open(archive, "r:gz") as packaged:
                self.assertEqual(
                    {member.name: member.mode for member in packaged},
                    {
                        "bin/linux-x64/bsl-analyzer": 0o755,
                        "bin/linux-x64/lib/libpython3.12.so.1.0": 0o644,
                        "bin/linux-x64/tool-index": 0o755,
                        "bin/linux-x64/tool-server": 0o755,
                    },
                )

    def test_stages_complete_selected_artifacts_and_excludes_other_tools(self) -> None:
        windows = os.name == "nt"
        target = "bin/win-x64" if windows else "bin/linux-x64"
        extension = ".exe" if windows else ""
        library = f"{target}/lib/" + ("python312.dll" if windows else "libpython3.12.so.1.0")
        entrypoint = f"{target}/tool-server{extension}"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root, windows=windows)
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
                    "tools-archive",
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
                    f"{target}/bsl-analyzer{extension}",
                    library,
                    f"{target}/tool-index{extension}",
                    entrypoint,
                ],
            )
            self.assertEqual((out_dir / library).read_bytes(), b"archive-library")
            if windows:
                self.assertEqual(run_windows_engine(out_dir / entrypoint), "assessment-engine")
            else:
                self.assertEqual(stat.S_IMODE((out_dir / entrypoint).stat().st_mode), 0o755)
                self.assertEqual(stat.S_IMODE((out_dir / library).stat().st_mode), 0o644)

    def test_rejects_a_tampered_selected_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root)
            (bundle / "bin/linux-x64/tool-server").write_bytes(b"tampered")

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--bundle-root",
                    str(bundle),
                    "--out-dir",
                    str(root / "assessment-engine"),
                    "--artifact",
                    "tools-archive",
                ],
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("size mismatch", result.stderr)


if __name__ == "__main__":
    unittest.main()
