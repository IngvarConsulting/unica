"""Focused normative guard for the active module event catalog invariant."""

from __future__ import annotations

import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
INVARIANT = REPO_ROOT / "arch" / "invariants" / "INV.PLATFORM.MODULE-EVENT-CATALOG.md"


class ModuleEventCatalogContractTests(unittest.TestCase):
    def test_form_event_ownership_clause_names_the_closed_owner_set(self) -> None:
        text = " ".join(INVARIANT.read_text(encoding="utf-8").split())
        clause = re.search(r"События формы остаются на ([^;]+);", text)

        self.assertIsNotNone(
            clause,
            "active invariant must contain a grammatical form-event ownership clause",
        )
        owners = tuple(re.split(r",\s*|\s+или\s+", clause.group(1)))
        self.assertEqual(
            owners,
            ("форме", "элементе", "таблице", "вложенной колонке", "команде"),
        )


if __name__ == "__main__":
    unittest.main()
