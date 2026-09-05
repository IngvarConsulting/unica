"""Страж размера: выражение `medium` покрывает каждый файл с процессом или сокетом.

Страж не знает наших имён: признак — конструкции стандартной библиотеки и
Cargo. Выражение объявлено в `.config/nextest.toml` дважды — воротами `pr`
и сроком `medium` — и обе копии обязаны совпадать.
"""

from __future__ import annotations

import importlib.util
import re
import tomllib
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
NEXTEST_TOML = REPO_ROOT / ".config" / "nextest.toml"


def load_size_filters():
    spec = importlib.util.spec_from_file_location("size_filters", REPO_ROOT / "scripts" / "ci" / "size-filters.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SizeGuardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.config = tomllib.loads(NEXTEST_TOML.read_text(encoding="utf-8"))
        self.deadline = next(o for o in self.config["profile"]["default"]["overrides"] if "slow-timeout" in o)
        self.medium = self.deadline["filter"].strip()

    def test_pr_gate_and_medium_deadline_share_one_expression(self) -> None:
        pr = self.config["profile"]["pr"]["default-filter"].strip()

        self.assertEqual(pr, f"not (\n{self.medium}\n)")
        self.assertEqual(self.deadline["slow-timeout"], {"period": "300s", "terminate-after": 2})
        self.assertTrue(self.medium.startswith("kind(test)"))

    def test_every_module_with_a_process_or_socket_is_declared_medium(self) -> None:
        """Файл с `std::process` или `std::net` не бывает `small` молча."""
        module = load_size_filters()
        declared = set(re.findall(r"test\(/\^([A-Za-z0-9_:]+)::", self.medium))

        for crate, path in module.flagged_modules(REPO_ROOT):
            with self.subTest(module=f"{crate}::{path}"):
                self.assertIn(path, declared)


if __name__ == "__main__":
    unittest.main()
