from __future__ import annotations

import json
import re
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

    def test_subsystem_result_owns_structure_not_object_memberships(self) -> None:
        runtime = (
            REPO_ROOT
            / "crates/unica-coder/src/infrastructure/native_operations/subsystem.rs"
        ).read_text(encoding="utf-8")
        skill = (
            REPO_ROOT / "plugins/unica/skills/subsystem-info/SKILL.md"
        ).read_text(encoding="utf-8")
        application = (
            REPO_ROOT / "crates/unica-coder/src/application/mod.rs"
        ).read_text(encoding="utf-8")

        answer = runtime.split("pub(crate) enum SubsystemInfoAnswer", 1)[1].split(
            "pub(crate) struct SubsystemGroupData", 1
        )[0]
        self.assertIn("Tree { tree: Vec<SubsystemTreeNode> }", answer)
        self.assertNotIn("functional_subsystems", answer)
        self.assertNotIn("interface_subsystems", answer)
        self.assertIn("pub(crate) tree: Option<Vec<SubsystemTreeNode>>", runtime)
        self.assertNotIn("functionalSubsystems", skill)
        self.assertNotIn("interfaceSubsystems", skill)
        for marker in (
            "цепоч",
            "корня",
            "потом",
            "самостоятельный XML",
            "локальн",
            "provider_unavailable",
            "отмен",
        ):
            self.assertIn(marker, skill)
        self.assertNotIn("`overview`", skill)
        self.assertNotIn("`Mode`", skill)
        self.assertNotIn("её каталог", skill)
        self.assertNotIn(
            "файл или каталог подсистемы дают её\nописание и структурный контекст",
            skill,
        )
        for exact_target in (
            "каталог `Subsystems`",
            "зарегистрированный XML",
            "самостоятельный незарегистрированный XML",
        ):
            self.assertIn(exact_target, skill)
        self.assertNotIn("full or focused registered tree", application)

        tool_contracts = (
            REPO_ROOT / "crates/unica-coder/src/application/tool_contracts.rs"
        ).read_text(encoding="utf-8")
        self.assertNotIn("whole `Subsystems/` folder for `Mode=tree`", tool_contracts)
        for marker in ("ADR-0033", "INV-SOURCE-SUBSYSTEM-TOPOLOGY"):
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

        ledger_section = ledger.split("### `unica.subsystem.info`", 1)[1].split(
            "### `unica.subsystem.validate`", 1
        )[0]
        for text in (ledger_section, review_text):
            for marker in (
                "tree",
                "ADR-0033",
                "INV-SOURCE-SUBSYSTEM-TOPOLOGY",
                "самостоятельн",
                "локальн",
                "provider_unavailable",
            ):
                self.assertIn(marker, text)
            self.assertNotIn("functionalSubsystems", text)
            self.assertNotIn("interfaceSubsystems", text)
            self.assertIn("цепоч", text)

    def test_skill_examples_do_not_advertise_nested_subsystems_directories(
        self,
    ) -> None:
        skill = (
            REPO_ROOT / "plugins/unica/skills/subsystem-info/SKILL.md"
        ).read_text(encoding="utf-8")
        targets = re.findall(r'"SubsystemPath"\s*:\s*"([^"]+)"', skill)
        directory_targets = {
            target.rstrip("/")
            for target in targets
            if not target.rstrip("/").endswith(".xml")
        }

        self.assertEqual(directory_targets, {"Subsystems"})
        self.assertIsNone(
            re.search(
                r"(?:каталог.{0,120}поддерев|поддерев.{0,120}каталог)",
                skill,
                flags=re.IGNORECASE | re.DOTALL,
            )
        )

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
        self.assertNotIn(
            "Subsystem.СтандартныеПодсистемы.Subsystem.Обсуждения", skill
        )


if __name__ == "__main__":
    unittest.main()
