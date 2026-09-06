"""Линия прогона: ветка как есть, тег — только на релизной линии."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "resolve-line.py"


def load_module():
    spec = importlib.util.spec_from_file_location("resolve_line", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ResolveLineTests(unittest.TestCase):
    def test_branch_push_is_its_own_line(self) -> None:
        module = load_module()

        self.assertEqual(module.resolve("branch", "release-v0.12", "abc", lambda sha: []), "release-v0.12")
        self.assertEqual(module.resolve("branch", "main", "abc", lambda sha: []), "main")

    def test_tag_resolves_to_the_youngest_release_line_holding_its_commit(self) -> None:
        module = load_module()
        containing = lambda sha: ["main", "release-v0.13", "release-v0.12", "feature/x"]

        self.assertEqual(module.resolve("tag", "v0.12.4", "abc", containing), "release-v0.12")

    def test_tag_outside_any_release_line_is_a_refusal(self) -> None:
        """Теги только на релизных линиях: тег на main или в воздухе — отказ."""
        module = load_module()

        with self.assertRaises(SystemExit) as refused:
            module.resolve("tag", "v0.14.0", "abc1234", lambda sha: ["main"])
        self.assertIn("релизной линии", str(refused.exception))


if __name__ == "__main__":
    unittest.main()
