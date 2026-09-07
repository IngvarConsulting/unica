"""Страница статуса: какие линии на ней стоят."""

from __future__ import annotations

import importlib.util
import unittest
from datetime import datetime, timezone
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "site-status.py"


def load_module():
    spec = importlib.util.spec_from_file_location("site_status", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SiteLinesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()
        self.module.open_lines = lambda repo, now: ["release-v0.12", "release-v0.13"]
        self.now = datetime.now(timezone.utc)

    def lines(self, branch: str) -> list[str]:
        return self.module.site_lines(branch, "IngvarConsulting/unica", self.now)

    def test_main_stays_on_the_page_whatever_line_the_run_came_from(self) -> None:
        self.assertEqual(self.lines("main"), ["main", "release-v0.12", "release-v0.13"])
        self.assertEqual(self.lines("release-v0.13"), ["main", "release-v0.13", "release-v0.12"])

    def test_a_tag_or_a_stray_branch_is_not_a_line_and_gets_no_card(self) -> None:
        """Результаты тега лежат в его линии; карточка «v0.13.2 — нет прогонов» врала бы."""
        for ref in ("v0.13.2", "feature/x", "release-v0.13.1"):
            with self.subTest(ref=ref):
                self.assertEqual(self.lines(ref), ["main", "release-v0.12", "release-v0.13"])


if __name__ == "__main__":
    unittest.main()
