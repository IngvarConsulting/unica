"""Cross-document contract for the delivered project health inspection."""

from __future__ import annotations

import json
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


class ProjectHealthContractTests(unittest.TestCase):
    def read(self, relative: str) -> str:
        return (REPO_ROOT / relative).read_text(encoding="utf-8")

    def test_project_health_contract_is_accepted_and_routed(self) -> None:
        surface_contract = self.read("arch/contracts/CTR.WIRE.TOOL-SURFACE.md")
        surface = self.read("arch/tool-surface.md")
        review = json.loads(
            self.read("arch/tool-surface-review.json")
        )
        workflow = self.read(
            "plugins/unica/references/use-cases/workspace-runtime.md"
        )
        self.assertIn("CTR.WIRE.TOOL-SURFACE", surface_contract)
        self.assertIn("tests/ci/test_tool_surface_ledger.py", surface_contract)

        status_contract = review["unica.project.status"]["result"]
        self.assertEqual(status_contract["contract"], "typed")
        self.assertEqual(status_contract["target"], "достигнут")
        status_result = status_contract["now"]
        self.assertIn("ready", status_result)
        self.assertIn("repositoryReady", status_result)
        self.assertIn("`array`", status_result)
        self.assertIn("`null`", status_result)
        for contract in (surface, status_result, workflow):
            self.assertIn("working-tree", contract)
            self.assertIn("staged index", contract)
        self.assertIn("unica.project.status", surface)
        self.assertIn("unica.project.map", surface)

        map_review = json.dumps(
            review["unica.project.map"], ensure_ascii=False
        ).lower()
        self.assertNotIn("health", map_review)
        self.assertNotIn("готов", map_review)

        self.assertIn("unica.project.status", workflow)
        self.assertIn("ready", workflow)
        self.assertIn("repositoryReady", workflow)
        self.assertIn("otherwise `null`", workflow)
        self.assertIn("remediation", workflow)

if __name__ == "__main__":
    unittest.main()
