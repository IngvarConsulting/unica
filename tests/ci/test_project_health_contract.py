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
        adr = self.read(
            "spec/decisions/0060-project-status-publikuet-gotovnost-proekta.md"
        )
        invariants = self.read("spec/architecture/invariants.md")
        decisions = self.read("spec/decisions/README.md")
        surface = self.read("spec/architecture/tool-surface.md")
        review = json.loads(
            self.read("spec/architecture/tool-surface-review.json")
        )
        workflow = self.read(
            "plugins/unica/references/use-cases/workspace-runtime.md"
        )
        implementation_plan = self.read(
            "docs/plans/2026-08-13-project-source-health.md"
        )
        self.assertIn("- Статус: `accepted`", adr)
        self.assertIn("ADR-0060", implementation_plan)
        for invariant in (
            "INV-MCP-PROJECT-READINESS",
            "INV-SOURCE-ROOT-SEPARATION",
            "INV-SOURCE-PORTABLE-GIT",
        ):
            self.assertIn(f"### {invariant}", invariants)

        accepted, proposed = decisions.split("## Предложенные решения", maxsplit=1)
        self.assertIn("ADR-0060", accepted)
        self.assertNotIn("ADR-0060", proposed)

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

    def test_single_resolved_root_does_not_narrow_project_inspection(self) -> None:
        invariants = self.read("spec/architecture/invariants.md")
        resolved_root = invariants.split(
            "### INV-SOURCE-SINGLE-RESOLVED-ROOT", maxsplit=1
        )[1].split("\n### ", maxsplit=1)[0]
        separated_root = invariants.split(
            "### INV-SOURCE-ROOT-SEPARATION", maxsplit=1
        )[1].split("\n### ", maxsplit=1)[0]

        self.assertRegex(
            resolved_root,
            r"кажд(?:ый|ого)\s+уникально адресуем(?:ый|ого)\s+набор(?:а)?",
        )
        self.assertRegex(
            resolved_root,
            r"не создаёт\s+неразличимые проверки с\s+`sourceSet`",
        )
        self.assertNotIn(
            "`unica.project.status` и `unica.project.map`", resolved_root
        )
        self.assertIn("каждый уникально адресуемый корень", separated_root)


if __name__ == "__main__":
    unittest.main()
