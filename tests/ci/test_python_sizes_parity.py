"""Объявления размеров в модулях повторяют `.config/python-sizes.toml` один в один.

Переходный тест: доказывает равенство перед удалением манифеста.
"""

from __future__ import annotations

import importlib.util
import sys
import tomllib
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("python_sizes", REPO_ROOT / "scripts" / "ci" / "python_sizes.py")
sizes = importlib.util.module_from_spec(SPEC)
sys.modules.setdefault("python_sizes", sizes)
SPEC.loader.exec_module(sizes)


class PythonSizesParityTests(unittest.TestCase):
    def test_declarations_equal_the_manifest(self) -> None:
        manifest = tomllib.loads((REPO_ROOT / ".config" / "python-sizes.toml").read_text(encoding="utf-8"))
        declared = sizes.collect(REPO_ROOT)
        self.assertEqual(sorted(declared.medium), sorted(manifest["medium"]["entries"]))
        self.assertEqual(sorted(declared.pr_skip), sorted(manifest["pr"]["skip"]))
        self.assertEqual(sorted(declared.keep_medium), sorted(manifest["queue"]["keep-medium"]))


if __name__ == "__main__":
    unittest.main()
