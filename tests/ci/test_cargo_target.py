"""Каталог сборки скриптов: `CARGO_TARGET_DIR`, без неё — `target` workspace.

Скрипт, который сам называет `target` внутри рабочей копии, обходит каталог
сборки сессии и наполняет checkout. Страж держит это в одном месте:
`scripts/ci/cargo_target.py`.
"""
from __future__ import annotations

import importlib.util
import os
import re
import unittest
from pathlib import Path
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("cargo_target", REPO_ROOT / "scripts/ci/cargo_target.py")
cargo_target = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cargo_target)

# Literal build directories inside the working copy: `root / "target"`,
# `"target/…"` and a string argument right after `--target-dir`.
LITERAL_TARGET = re.compile(r'/\s*"target(?:/[^"]*)?"|"target/|"--target-dir",\s*"')
# Where a literal is the point: the helper itself, and the report that finds
# stray `target/` directories in working copies.
ALLOWED = {"scripts/ci/cargo_target.py", "scripts/dev/prune-build-caches.py"}
# A vendored selection keeps `--target-dir target` for CI; `targeted()` maps it.
MARKER = "# cargo-target: mapped"


class CargoTargetTests(unittest.TestCase):
    def test_the_session_variable_wins_and_its_absence_keeps_the_workspace_target(self):
        root = Path("/repo")
        with mock.patch.dict(os.environ):
            os.environ.pop("CARGO_TARGET_DIR", None)
            self.assertEqual(cargo_target.cargo_target_dir(root), root / "target")
            self.assertEqual(cargo_target.vendor_target_dir(root, "ureq"), root / "vendor/ureq/target")
            self.assertEqual(cargo_target.vendor_target_arg(root, "ureq"), "target")
            absolute = Path("/builds/session").resolve()
            os.environ["CARGO_TARGET_DIR"] = str(absolute)
            self.assertEqual(cargo_target.cargo_target_dir(root), absolute)
            self.assertEqual(cargo_target.vendor_target_dir(root, "ureq"), absolute / "ureq")
            self.assertEqual(cargo_target.vendor_target_arg(root, "ureq"), str(absolute / "ureq"))
            os.environ["CARGO_TARGET_DIR"] = "out"
            self.assertEqual(cargo_target.cargo_target_dir(root), root / "out")

    def test_scripts_take_the_build_directory_from_the_helper(self):
        offenders = []
        for path in sorted((REPO_ROOT / "scripts").rglob("*.py")):
            relative = path.relative_to(REPO_ROOT).as_posix()
            if relative in ALLOWED:
                continue
            for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                if LITERAL_TARGET.search(line) and MARKER not in line:
                    offenders.append(f"{relative}:{number}: {line.strip()}")
        self.assertEqual(offenders, [], "build directories come from scripts/ci/cargo_target.py")

    def test_the_guard_sees_each_literal_shape(self):
        for line in ('root / "target"', 'REPO_ROOT / "target/debug/unica"', '"--target-dir", "target",',
                     'root / "target/nextest" / profile'):
            with self.subTest(line=line):
                self.assertTrue(LITERAL_TARGET.search(line))
        for line in ('manifest["target"]', '"--target-dir", str(cargo_target_dir(root))', 'metadata.get("target")'):
            with self.subTest(line=line):
                self.assertFalse(LITERAL_TARGET.search(line))


if __name__ == "__main__":
    unittest.main()
