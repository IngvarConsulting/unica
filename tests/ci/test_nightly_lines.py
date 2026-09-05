"""Ночной прогон идёт только туда, где вершина линии сдвинулась."""

from __future__ import annotations

import importlib.util
import unittest
from datetime import datetime, timezone
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "nightly-lines.py"


def load_module():
    spec = importlib.util.spec_from_file_location("nightly_lines", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class NightlyLinesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()
        self.heads = {"main": "aaaa111" + "0" * 33, "release-v0.12": "bbbb222" + "0" * 33}
        self.memory = {
            "https://site/data/main/profiles/large.json": {"sha": self.heads["main"], "at": "2026-09-04T01:30:00Z"},
            "https://site/data/release-v0.12/profiles/large.json": {"sha": "cccc333" + "0" * 33, "at": "2026-08-30T01:30:00Z"},
        }

    def decisions(self, memory=None):
        memory = self.memory if memory is None else memory
        return self.module.enumerate_lines(
            "IngvarConsulting/unica",
            "https://site",
            datetime(2026, 9, 5, tzinfo=timezone.utc),
            gh=lambda repo, path: {"commit": {"sha": self.heads[path.removeprefix("branches/")]}},
            fetch=lambda url: memory.get(url),
            open_lines=lambda repo, now: ["release-v0.12"],
        )

    def test_unmoved_line_is_skipped_and_moved_line_runs(self) -> None:
        decisions = {d["line"]: d for d in self.decisions()}

        self.assertFalse(decisions["main"]["run"])
        self.assertIn("вершина на месте", decisions["main"]["reason"])
        self.assertTrue(decisions["release-v0.12"]["run"])
        self.assertIn("cccc333 → bbbb222", decisions["release-v0.12"]["reason"])

    def test_line_without_memory_runs(self) -> None:
        decisions = {d["line"]: d for d in self.decisions(memory={})}

        self.assertTrue(all(d["run"] for d in decisions.values()))
        self.assertIn("памяти", decisions["main"]["reason"])

    def test_matrix_holds_full_line_by_runner_combinations(self) -> None:
        chosen = self.module.matrix(self.decisions(), runners=("ubuntu-latest", "macos-14"))

        self.assertEqual(
            chosen["include"],
            [
                {"line": "release-v0.12", "sha": self.heads["release-v0.12"], "runner": "ubuntu-latest"},
                {"line": "release-v0.12", "sha": self.heads["release-v0.12"], "runner": "macos-14"},
            ],
        )

    def test_empty_matrix_when_nothing_moved(self) -> None:
        memory = dict(self.memory)
        memory["https://site/data/release-v0.12/profiles/large.json"] = {"sha": self.heads["release-v0.12"], "at": "x"}

        self.assertEqual(self.module.matrix(self.decisions(memory=memory))["include"], [])


if __name__ == "__main__":
    unittest.main()
