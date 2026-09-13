"""Страж состава: ворота, профили nextest и размеры наборов согласованы.

Отбор объявлен профилями: `pr` пропускает только `small`, очередь и `main` —
всё, кроме `large`, релиз гоняет всё, `large` — названные тесты ёмкости и
нагрузки контракта ReceiptLedger, а на Windows — весь набор. Меняется этот
страж осознанно, вместе с выражениями профилей.
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

        large = profiles["large"]["default-filter"].strip()
        for gate in GATES:
            with self.subTest(gate=gate):
                self.assertIn(gate, profiles)
                self.assertEqual(self.run_tests.nextest_profile(gate), gate)
                if gate == "pr":
                    # Pull request — только small: всё, что не объявлено medium.
                    self.assertTrue(profiles[gate]["default-filter"].startswith("not ("))
                elif gate == "release":
                    self.assertEqual(profiles[gate].get("default-filter"), "all()")
                else:
                    # Очередь и `main` — всё, кроме ночного яруса, тем же выражением.
                    self.assertEqual(profiles[gate]["default-filter"].strip(), f"not (\n{large}\n)")
        self.assertEqual(set(self.run_tests.PROFILES), {"all", "large", *GATES})
        # Ночной ярус на ubuntu и macOS — только названные тесты одной цели;
        # тот же список стоит и у срока `large`.
        self.assertTrue(large.startswith("binary(daemon_receipt_ledger) & (\n    test(/^"))
        deadline = next(o for o in profiles["default"]["overrides"] if o.get("threads-required"))
        self.assertEqual(deadline["filter"].strip(), large)
        self.assertEqual(deadline["slow-timeout"], {"period": "900s", "terminate-after": 2})
        self.assertEqual(deadline["threads-required"], "num-cpus")
        # Ночью Windows гоняет всё, кроме детерминированной модели горизонта
        # нагрузки: она платформе безразлична и на двух ядрах не укладывается
        # в два срока `large`.
        self.assertEqual(
            profiles["large"]["overrides"],
            [{
                "platform": "cfg(windows)",
                "default-filter": "all() - test(/^deterministic_horizon_load_does_not_saturate$/)",
            }],
        )
        for command in self.run_tests.rust_commands("large"):
            self.assertIn("--no-tests=pass", command)

    def test_large_tier_names_only_tests_that_exist_in_the_ledger_contract(self) -> None:
        """Ярус объявлен именами: переименованный тест выпал бы из ночи молча."""
        import re

        large = self.config["profile"]["large"]["default-filter"]
        # Регулярное выражение nextest читается буквально, перенос строки в
        # нём — символ имени; поэтому ярус — по одному `test(/^имя$/)` на строку.
        names = re.findall(r"test\(/\^(\w+)\$/\)", large)
        self.assertEqual(len(names), large.count("test("))
        source = (REPO_ROOT / "crates" / "unica-coder" / "tests" / "daemon_receipt_ledger.rs").read_text(encoding="utf-8")

        self.assertEqual(names, sorted(names))
        for name in names:
            with self.subTest(test=name.strip()):
                self.assertIsNotNone(
                    re.search(rf"^fn {name.strip()}\(\)", source, re.M), "тест яруса large не найден в контракте"
                )

    def test_python_matrix_names_every_suite_of_the_seam_and_lanes_only_admitted_sizes(self) -> None:
        """Матрица ворот: каждый набор шва, полосатый — по допущенным размерам, не больше."""
        for gate in ("pr", "queue", "main", "release"):
            with self.subTest(gate=gate):
                matrix = self.run_tests.python_matrix(gate)
                self.assertEqual(sorted({entry["suite"] for entry in matrix}), sorted(suite for suite, _, _ in self.run_tests.PYTHON_SUITES))
                lanes = {entry["lane"] for entry in matrix if entry["suite"] in self.run_tests.LANED_SUITES}
                self.assertEqual(lanes, set(self.run_tests.ADMITTED[gate]))
                self.assertTrue(all(not entry["lane"] for entry in matrix if entry["suite"] not in self.run_tests.LANED_SUITES))

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
