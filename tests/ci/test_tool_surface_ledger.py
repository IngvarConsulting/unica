"""The tool-surface ledger must describe the registry that actually ships.

A hand-maintained inventory of 71 tools drifts on the first merge, so the
mechanical columns are generated and this guard fails when they stop matching
the built binary or when a tool has no review entry.
"""

from __future__ import annotations

import collections
import importlib.util
import json
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
GENERATOR = REPO_ROOT / "scripts/ci/generate-tool-surface.py"
LEDGER = REPO_ROOT / "spec/architecture/tool-surface.md"
REVIEW = REPO_ROOT / "spec/architecture/tool-surface-review.json"
BINARY = REPO_ROOT / "target/debug/unica"


def load_generator():
    spec = importlib.util.spec_from_file_location("unica_tool_surface", GENERATOR)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ToolSurfaceLedgerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        subprocess.run(
            ["cargo", "build", "--quiet", "--package", "unica-coder", "--bin", "unica"],
            cwd=REPO_ROOT,
            check=True,
        )
        cls.module = load_generator()
        cls.tools = cls.module.read_registry(BINARY)
        cls.review = json.loads(REVIEW.read_text(encoding="utf-8"))

    def test_every_published_tool_has_a_review_entry(self) -> None:
        published = {tool["name"] for tool in self.tools}
        self.assertEqual(published - set(self.review), set())
        self.assertEqual(set(self.review) - published, set())

    def test_every_review_entry_states_a_contract_and_scenarios(self) -> None:
        for name, entry in sorted(self.review.items()):
            with self.subTest(tool=name):
                # The migration metric reads this field, never the free-text
                # note beside it: counting progress by matching substrings in
                # prose is the very mistake ADR-0023 removes from the tools.
                self.assertIn(entry["result"]["contract"], self.module.CONTRACT_STATES)
                self.assertTrue(entry["result"]["now"].strip(), name)
                self.assertTrue(entry["result"]["target"].strip(), name)
                # One scenario documents a tool nobody reviewed against real
                # use; the point of the ledger is more than a restated summary.
                self.assertGreaterEqual(len(entry["scenarios"]), 1, name)
                for scenario in entry["scenarios"]:
                    self.assertTrue(scenario.strip(), name)

    def test_ledger_matches_the_live_registry(self) -> None:
        result = subprocess.run(
            [sys.executable, str(GENERATOR), "--check", "--binary", str(BINARY)],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)

    def test_ledger_counts_the_migration_it_tracks(self) -> None:
        text = LEDGER.read_text(encoding="utf-8")
        self.assertIn(f"- Инструментов: **{len(self.tools)}**", text)
        states = collections.Counter(
            entry["result"]["contract"] for entry in self.review.values()
        )
        self.assertEqual(sum(states.values()), len(self.tools))
        for state, title in self.module.CONTRACT_STATES.items():
            self.assertIn(f"- {title}: **{states[state]}**", text)
        remaining = states["prose"] + states["partial"]
        self.assertIn(f"типизированный `data`: **{remaining}**", text)

    def test_a_partially_typed_tool_is_not_counted_as_migrated(self) -> None:
        """meta.info puts its resolved address in `data` and its report in
        prose. Counting it as done hid a tool that still has to move."""

        self.assertEqual(self.review["unica.meta.info"]["result"]["contract"], "partial")


if __name__ == "__main__":
    unittest.main()
