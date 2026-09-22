"""Исследование — не тест: код собирается только с фичей `research`, обёртки — в scripts/research."""

from __future__ import annotations

import tomllib
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
CRATE = REPO_ROOT / "crates" / "unica-coder"


class ResearchPolicyTests(unittest.TestCase):
    def test_research_lives_outside_the_test_plan(self) -> None:
        """Цели исследований требуют фичу `research`; в плане конвейера их нет."""
        cargo = tomllib.loads((CRATE / "Cargo.toml").read_text(encoding="utf-8"))

        self.assertIn("research", cargo["features"])
        targets = [target for target in cargo.get("test", []) if target["path"].startswith("tests/research/")]
        self.assertGreaterEqual(len(targets), 2)
        for target in targets:
            with self.subTest(target=target["name"]):
                self.assertEqual(target.get("required-features"), ["research"])
                self.assertTrue((CRATE / target["path"]).is_file())

    def test_research_wrappers_enable_the_research_feature(self) -> None:
        wrappers = sorted((REPO_ROOT / "scripts" / "research").glob("*.sh"))

        self.assertGreaterEqual(len(wrappers), 3)
        for wrapper in wrappers:
            with self.subTest(wrapper=wrapper.name):
                text = wrapper.read_text(encoding="utf-8")
                self.assertIn("--features research", text)


if __name__ == "__main__":
    unittest.main()
