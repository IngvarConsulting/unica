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
            "spec/decisions/0056-project-status-publikuet-gotovnost-proekta.md"
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

        self.assertIn("- Статус: `accepted`", adr)
        self.assertIn(
            "`sourceSets[].sourceFormat` остаётся результатом проверки рабочего дерева",
            adr,
        )
        self.assertRegex(adr, r"сама\s+проверка индекс не изменяет")
        self.assertRegex(
            adr,
            r"кажд(?:ый|ого)\s+уникально адресуем(?:ый|ого)\s+набор(?:а)?",
        )
        self.assertRegex(adr, r"не создаёт неразличимые проверки с\s+`sourceSet`")
        for invariant in (
            "INV-MCP-PROJECT-READINESS",
            "INV-SOURCE-ROOT-SEPARATION",
            "INV-SOURCE-PORTABLE-GIT",
        ):
            self.assertIn(f"### {invariant}", invariants)

        accepted, proposed = decisions.split("## Предложенные решения", maxsplit=1)
        self.assertIn("ADR-0056", accepted)
        self.assertNotIn("ADR-0056", proposed)

        status_result = review["unica.project.status"]["result"]["now"]
        self.assertIn("ready", status_result)
        self.assertIn("repositoryReady", status_result)
        self.assertIn("`array`", status_result)
        self.assertIn("`null`", status_result)
        for contract in (surface, status_result, workflow):
            self.assertIn("working-tree", contract)
            self.assertIn("staged index", contract)

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


if __name__ == "__main__":
    unittest.main()
