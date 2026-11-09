"""Checks the current compatibility surface and shipped tool guidance."""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
LEDGER = REPO_ROOT / "docs/tool-surface.md"
REVIEW = REPO_ROOT / "tests/fixtures/v013/tool-surface-review.json"
CANONICAL = {
    "unica.view",
    "unica.apply",
    "unica.resolve",
    "unica.search",
    "unica.check",
    "unica.diff",
    "unica.run",
    "unica.docs",
}
COMPATIBILITY = {
    "unica.task.get",
    "unica.task.result",
    "unica.task.cancel",
}


class SurfaceFirstTransitionMatrixTests(unittest.TestCase):
    def test_generated_ledger_and_review_are_the_exact_compatibility_profile(self) -> None:
        ledger = LEDGER.read_text(encoding="utf-8")
        names = set(re.findall(r"^### `([^`]+)`$", ledger, re.MULTILINE))
        review = set(json.loads(REVIEW.read_text(encoding="utf-8")))

        self.assertEqual(names, CANONICAL | COMPATIBILITY)
        self.assertEqual(review, names)

if __name__ == "__main__":
    unittest.main()
