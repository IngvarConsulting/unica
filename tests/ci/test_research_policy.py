"""Исследование — не тест: код собирается только с фичей `research`, обёртки — в scripts/research."""

from __future__ import annotations

import re
import tomllib
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
CRATE = REPO_ROOT / "crates" / "unica-coder"
IGNORE_REASON = re.compile(r'#\[ignore = "([^"]*)"\]')


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

    def test_no_research_hides_behind_an_ignore_switch(self) -> None:
        """`#[ignore]` с причиной «writes …» — исследование под выключателем; таких не остаётся."""
        for path in (REPO_ROOT / "crates").rglob("*.rs"):
            if "target" in path.parts:
                continue
            for found in IGNORE_REASON.finditer(path.read_text(encoding="utf-8", errors="replace")):
                with self.subTest(path=str(path.relative_to(REPO_ROOT))):
                    self.assertFalse(found.group(1).startswith("writes "), found.group(1))

    def test_every_wrapper_states_question_method_and_the_research_feature(self) -> None:
        wrappers = sorted((REPO_ROOT / "scripts" / "research").glob("*.sh"))

        self.assertGreaterEqual(len(wrappers), 3)
        for wrapper in wrappers:
            with self.subTest(wrapper=wrapper.name):
                text = wrapper.read_text(encoding="utf-8")
                self.assertIn("# Вопрос:", text)
                self.assertIn("# Метод:", text)
                self.assertIn("--features research", text)


if __name__ == "__main__":
    unittest.main()
