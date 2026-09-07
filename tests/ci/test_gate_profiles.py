"""Страж состава: ворота, профили nextest и размеры наборов согласованы.

Отбор объявлен профилями: `pr` пропускает только `small`, очередь, `main` и
релиз гоняют всё, `large` пуст везде, кроме Windows. Меняется этот страж
осознанно, вместе с выражениями профилей.
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

    def test_every_gate_has_a_nextest_profile_that_admits_its_sizes(self) -> None:
        profiles = self.config["profile"]

        for gate in GATES:
            with self.subTest(gate=gate):
                self.assertIn(gate, profiles)
                self.assertEqual(self.run_tests.nextest_profile(gate), gate)
                if gate == "pr":
                    # Pull request — только small: всё, что не объявлено medium.
                    self.assertTrue(profiles[gate]["default-filter"].startswith("not ("))
                else:
                    self.assertEqual(profiles[gate].get("default-filter"), "all()")
        self.assertEqual(set(self.run_tests.PROFILES), {"all", "large", *GATES})
        # Ночной ярус пуст на ubuntu и macOS: их набор целиком идёт в `main`.
        self.assertEqual(profiles["large"].get("default-filter"), "none()")
        # Ночью Windows гоняет всё: переопределение по платформе в профиле large.
        self.assertEqual(profiles["large"]["overrides"], [{"platform": "cfg(windows)", "default-filter": "all()"}])
        self.assertIn("--no-tests=pass", self.run_tests.rust_commands("large")[0])

    def test_python_matrix_in_the_workflow_names_every_suite_of_the_seam(self) -> None:
        """Джоба на набор: матрица workflow и `PYTHON_SUITES` шва — один список."""
        import yaml

        release = yaml.safe_load((REPO_ROOT / ".github" / "workflows" / "unica-plugin-release.yml").read_text(encoding="utf-8"))
        matrix = release["jobs"]["test-python"]["strategy"]["matrix"]["include"]

        self.assertEqual([entry["suite"] for entry in matrix], [suite for suite, _, _ in self.run_tests.PYTHON_SUITES])
        self.assertEqual([entry["slug"] for entry in matrix], [suite.split("/")[-1] for suite, _, _ in self.run_tests.PYTHON_SUITES])

    def test_nextest_version_in_config_matches_the_workflow_install(self) -> None:
        """Одна версия исполнителя для CI и локального прогона."""
        import yaml

        release = yaml.safe_load((REPO_ROOT / ".github" / "workflows" / "unica-plugin-release.yml").read_text(encoding="utf-8"))
        tools = next(
            step["with"]["tool"]
            for step in release["jobs"]["test-rust-platforms"]["steps"]
            if step.get("uses", "").startswith("taiki-e/install-action@")
        )
        installed = next(part.split("@")[1] for part in tools.split(",") if part.startswith("cargo-nextest@"))

        self.assertEqual(self.config["nextest-version"], {"recommended": installed})

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
        # Размер набора — `small`; `medium` внутри набора объявляет манифест
        # `.config/python-sizes.toml`, и `pr` его не гоняет.
        self.assertEqual({size for _, size, _ in self.run_tests.PYTHON_SUITES}, {"small"})


if __name__ == "__main__":
    unittest.main()
