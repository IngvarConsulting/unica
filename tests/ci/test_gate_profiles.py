"""Страж состава: ворота, профили nextest и размеры наборов согласованы.

Отбора по размеру пока нет — каждый профиль гоняет всё. Когда размеры будут
расставлены, этот страж меняется осознанно, вместе с выражениями профилей.
"""

from __future__ import annotations

import importlib.util
import tomllib
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
GATES = ("pr", "queue", "main", "release")


def load_run_tests():
    spec = importlib.util.spec_from_file_location("run_tests", REPO_ROOT / "scripts" / "ci" / "run-tests.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class GateProfileCompositionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.config = tomllib.loads((REPO_ROOT / ".config" / "nextest.toml").read_text(encoding="utf-8"))
        self.run_tests = load_run_tests()

    def test_every_gate_has_a_nextest_profile_that_selects_everything(self) -> None:
        profiles = self.config["profile"]

        for gate in GATES:
            with self.subTest(gate=gate):
                self.assertIn(gate, profiles)
                self.assertEqual(profiles[gate].get("default-filter"), "all()")
                self.assertEqual(self.run_tests.nextest_profile(gate), gate)
        self.assertEqual(set(self.run_tests.PROFILES), {"all", "large", *GATES})
        # Ночной ярус пуст до расстановки размеров и честно гоняет ноль.
        self.assertEqual(profiles["large"].get("default-filter"), "none()")
        self.assertIn("--no-tests=pass", self.run_tests.rust_commands("large")[0])

    def test_default_profile_carries_the_small_deadline_for_everyone(self) -> None:
        """Срок на тест — то, чем размер держится честным; пока он один на всех."""
        default = self.config["profile"]["default"]

        self.assertEqual(default["slow-timeout"], {"period": "60s", "terminate-after": 2})
        self.assertEqual(default["junit"]["report-skipped"], "ignored")

    def test_python_suites_declare_a_size_the_gates_understand(self) -> None:
        sizes = set(self.run_tests.SIZES)

        self.assertEqual(set(self.run_tests.ADMITTED), set(self.run_tests.PROFILES))
        for gate, admitted in self.run_tests.ADMITTED.items():
            with self.subTest(gate=gate):
                self.assertTrue(set(admitted) <= sizes)
        for suite, size, _ in self.run_tests.PYTHON_SUITES:
            with self.subTest(suite=suite):
                self.assertIn(size, sizes)
                self.assertTrue((REPO_ROOT / suite).is_dir())
        # Площадка без отбора: `pr` принимает только `small`, и пока это все наборы.
        self.assertEqual({size for _, size, _ in self.run_tests.PYTHON_SUITES}, {"small"})


if __name__ == "__main__":
    unittest.main()
