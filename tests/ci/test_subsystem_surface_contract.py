from __future__ import annotations

import json
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


class SubsystemSurfaceContractTests(unittest.TestCase):
    def test_address_is_separate_from_metadata_address(self) -> None:
        subsystem = (
            REPO_ROOT / "crates/unica-coder/src/domain/subsystem.rs"
        ).read_text(encoding="utf-8")
        source_target = (
            REPO_ROOT / "crates/unica-coder/src/domain/source_target.rs"
        ).read_text(encoding="utf-8")

        self.assertIn("pub struct SubsystemAddress", subsystem)
        self.assertIn("#[serde(transparent)]", subsystem)
        self.assertIn("SUBSYSTEM_ADDRESS_MAX_DEPTH", subsystem)
        self.assertIn('"СтандартныеПодсистемы.Обсуждения"', subsystem)
        self.assertNotIn("SUBSYSTEM_ADDRESS_MAX_SEGMENTS", source_target)
        self.assertIn('profile.parse("Subsystem.A.B").is_err()', source_target)

    def test_catalog_result_keeps_tree_and_two_flat_lists(self) -> None:
        runtime = (
            REPO_ROOT
            / "crates/unica-coder/src/infrastructure/native_operations/subsystem.rs"
        ).read_text(encoding="utf-8")
        skill = (
            REPO_ROOT / "plugins/unica/skills/subsystem-info/SKILL.md"
        ).read_text(encoding="utf-8")

        for marker in ("tree", "functionalSubsystems", "interfaceSubsystems"):
            self.assertIn(marker, runtime)
            self.assertIn(marker, skill)

        tool_contracts = (
            REPO_ROOT / "crates/unica-coder/src/application/tool_contracts.rs"
        ).read_text(encoding="utf-8")
        self.assertNotIn("whole `Subsystems/` folder for `Mode=tree`", tool_contracts)
        for marker in (
            "СтандартныеПодсистемы.Обсуждения",
            "ADR-0033",
            "INV-SOURCE-SUBSYSTEM-TOPOLOGY",
        ):
            self.assertIn(marker, skill)

    def test_surface_ledger_names_the_shared_registered_contract(self) -> None:
        ledger = (
            REPO_ROOT / "spec/architecture/tool-surface.md"
        ).read_text(encoding="utf-8")
        review = json.loads(
            (
                REPO_ROOT / "spec/architecture/tool-surface-review.json"
            ).read_text(encoding="utf-8")
        )["unica.subsystem.info"]
        review_text = json.dumps(review, ensure_ascii=False)

        for text in (ledger, review_text):
            for marker in (
                "tree",
                "functionalSubsystems",
                "interfaceSubsystems",
                "ADR-0033",
                "INV-SOURCE-SUBSYSTEM-TOPOLOGY",
            ):
                self.assertIn(marker, text)

    def test_bsp_address_does_not_replace_platform_xml_reference(self) -> None:
        specification = (
            REPO_ROOT
            / "plugins/unica/references/specs/1c-subsystem-spec.md"
        ).read_text(encoding="utf-8")
        skill = (
            REPO_ROOT / "plugins/unica/skills/subsystem-info/SKILL.md"
        ).read_text(encoding="utf-8")

        self.assertIn(
            "Subsystem.СтандартныеПодсистемы.Subsystem.Обсуждения",
            specification,
        )
        self.assertIn("СтандартныеПодсистемы.Обсуждения", skill)
        self.assertNotIn(
            "Subsystem.СтандартныеПодсистемы.Subsystem.Обсуждения", skill
        )


if __name__ == "__main__":
    unittest.main()
