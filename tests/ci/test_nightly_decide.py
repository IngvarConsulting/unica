"""Ночь гонит сдвинувшуюся вершину main и пропускает уже проверенную."""

from __future__ import annotations

import importlib.util
import io
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "nightly-decide.py"
SPEC = importlib.util.spec_from_file_location("nightly_decide", PATH)
assert SPEC is not None and SPEC.loader is not None
nightly = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(nightly)


class NightlyDecideTests(unittest.TestCase):
    def test_a_moved_tip_runs_and_a_checked_tip_skips(self) -> None:
        self.assertTrue(nightly.decide("b" * 40, {"sha": "a" * 40})[0])
        run, reason = nightly.decide("a" * 40, {"sha": "a" * 40, "at": "вчера"})
        self.assertFalse(run)
        self.assertIn("вчера", reason)

    def test_no_memory_runs_a_missing_night_costs_more_than_an_extra_one(self) -> None:
        self.assertTrue(nightly.decide("a" * 40, None)[0])
        self.assertTrue(nightly.decide("a" * 40, {})[0])

    def test_force_runs_a_checked_tip(self) -> None:
        self.assertTrue(nightly.decide("a" * 40, {"sha": "a" * 40}, force=True)[0])

    def test_an_unreadable_site_is_no_memory(self) -> None:
        with patch.object(nightly.urllib.request, "urlopen", side_effect=nightly.urllib.error.URLError("down")):
            self.assertIsNone(nightly.fetch_json("https://example.invalid/x.json"))

    def test_the_decision_goes_to_github_output_and_the_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            summary = Path(temporary) / "summary.md"
            out, err = io.StringIO(), io.StringIO()
            with patch.object(nightly, "fetch_json", return_value={"sha": "a" * 40}), redirect_stdout(out), redirect_stderr(err):
                code = nightly.main(["--site", "https://example.invalid", "--sha", "a" * 40, "--summary", str(summary)])
            self.assertEqual(code, 0)
            self.assertEqual(out.getvalue().strip(), "run=false")
            self.assertIn("пропуск", summary.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
