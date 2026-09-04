from __future__ import annotations

import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


def record(record_id: str) -> str:
    return (REPO_ROOT / "arch" / "invariants" / f"{record_id}.md").read_text(
        encoding="utf-8"
    )


def check(record_id: str) -> str:
    """Имена проверок записи одной строкой.

    Проп несёт либо одно имя, либо блочный список: правило, которое держат
    несколько проверок, называет их все, а не обёртку над ними.
    """
    text = record(record_id)
    inline = re.search(r"^check: (\S.*)$", text, re.MULTILINE)
    if inline is not None:
        return inline.group(1)
    block = re.search(r"^check:[ \t]*\n((?:[ \t]+- .+\n)+)", text, re.MULTILINE)
    if block is None:
        raise AssertionError(f"{record_id} has no check")
    return " ".join(line.strip()[2:] for line in block.group(1).splitlines())


class SourceFateSemanticClosureTests(unittest.TestCase):
    def test_adr_0016_is_superseded_by_explicitly_narrower_profile_decision(self) -> None:
        fate = (REPO_ROOT / "docs" / "arch-v1" / "FATE.md").read_text(
            encoding="utf-8"
        )
        row = next(line for line in fate.splitlines() if "`ADR-0016`" in line)
        self.assertEqual(
            row,
            "| `ADR-0016` | `superseded` | "
            "`DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE` | — |",
        )

        decision = (
            REPO_ROOT
            / "arch"
            / "decisions"
            / "2026-08-21-single-writable-platform-xml-profile.md"
        ).read_text(encoding="utf-8")
        decision_words = " ".join(decision.split())
        self.assertIn(
            "самостоятельная норма platform-before-XSD не переносится",
            decision_words,
        )
        self.assertIn("нет сохранённой независимой пары", decision_words)
        self.assertIn("не создаёт нового проверяемого инварианта", decision_words)
        self.assertNotIn("INV.SOURCE.PLATFORM-BEFORE-XSD", decision)
        self.assertFalse(
            (REPO_ROOT / "arch" / "invariants" / "INV.SOURCE.PLATFORM-BEFORE-XSD.md").exists()
        )

    def test_profile_and_mutation_decisions_name_complete_aggregates(self) -> None:
        profile = (
            REPO_ROOT / "arch" / "decisions" /
            "2026-08-21-single-writable-platform-xml-profile.md"
        ).read_text(encoding="utf-8")
        mutation = (
            REPO_ROOT / "arch" / "decisions" /
            "2026-08-21-mutation-idempotence-scope.md"
        ).read_text(encoding="utf-8")
        for name in (
            "single_writable_platform_xml_profile_is_exact",
            "native_mutation_surface_has_exact_operations_and_schemas",
            "public_platform_xml_mutators_have_closed_pre_side_effect_format_refusal",
            "dcs_edit_blocks_old_external_source_set_via_owner_descriptor",
            "cf_init_public_guard_blocks_newer_existing_post_validation_dependency",
        ):
            self.assertIn(name, profile)
        self.assertNotIn("INV.SOURCE.PLATFORM-BEFORE-XSD", profile)
        for name in (
            "verified_public_mutator_idempotence_cases_are_exact",
            "repeated_interface_edit_preserves_identity_but_reports_attempted_update",
            "repeated_mxl_compile_preserves_identity_but_reports_attempted_update",
        ):
            self.assertIn(name, mutation)

    def test_portable_git_and_platform_xml_checks_are_closed_matrices(self) -> None:
        self.assertTrue(
            check("INV.SOURCE.PORTABLE-GIT").endswith(
                "::portable_git_readiness_contract_is_a_closed_positive_and_negative_matrix"
            )
        )
        self.assertTrue(
            check("INV.SOURCE.PLATFORM-XML-ONLY").endswith(
                "::native_platform_xml_source_format_public_gate_is_closed_over_public_operations"
            )
        )
        self.assertIn(
            "decision: DEC.2026-08-21.LEGACY-UNKNOWN-NATIVE-SOURCE-FORMAT",
            record("INV.SOURCE.PLATFORM-XML-ONLY"),
        )

    def test_mutator_rewrite_and_preimage_checks_use_the_closed_public_inventory(self) -> None:
        self.assertTrue(
            check("INV.SOURCE.IDEMPOTENT-REWRITE").endswith(
                "::verified_public_mutator_idempotence_cases_are_exact"
            )
        )
        self.assertTrue(
            check("INV.SOURCE.BOUND-PREIMAGES").endswith(
                "::public_platform_xml_mutator_preimage_contract_is_complete"
            )
        )

    def test_universal_idempotent_receipt_claim_is_retired_to_exact_current_behavior(self) -> None:
        fate = (REPO_ROOT / "docs" / "arch-v1" / "FATE.md").read_text(
            encoding="utf-8"
        )
        row = next(
            line
            for line in fate.splitlines()
            if "`INV-SOURCE-IDEMPOTENT-REWRITE`" in line
        )
        self.assertIn("`retired`", row)
        self.assertIn(
            "behavior-removed: DEC.2026-08-21.MUTATION-IDEMPOTENCE-SCOPE", row
        )
        decision = (
            REPO_ROOT
            / "arch"
            / "decisions"
            / "2026-08-21-mutation-idempotence-scope.md"
        ).read_text(encoding="utf-8")
        self.assertIn("unica.interface.edit", decision)
        self.assertIn("unica.mxl.compile", decision)
        self.assertIn("INV.SOURCE.IDEMPOTENT-ATTEMPT-METADATA", decision)

    def test_root_and_rollback_checks_execute_real_rejection_and_fault_paths(self) -> None:
        self.assertTrue(
            check("INV.SOURCE.ROOT-POLICIES-CLOSED").endswith(
                "::unknown_version_bearing_roots_are_rejected_by_the_closed_policy_catalog"
            )
        )
        named = check("INV.SOURCE.ROLLBACK-DIAGNOSTIC-CLASS")
        for name in (
            "registration_rollback_preserves_same_name_recovery_decoy_after_parent_swap",
            "registration_rollback_validation_reports_preserved_quarantine",
            "removal_rollback_preserves_concurrent_file_and_recovery_artifact",
            "removal_rollback_preserves_concurrent_empty_directory_and_recovery_tree",
            "successful_registration_cleanup_warns_and_preserves_decoy_after_parent_swap",
        ):
            self.assertIn(name, named)

    def test_subsystem_checks_cover_public_schema_and_no_data_failures(self) -> None:
        """Чтение подсистемы переехало на канонический путь.

        Логический адрес и отказ без данных доказываются приёмочным прогоном:
        снятый `subsystem.info` больше не может их держать.
        """
        corpus_check = (
            "tests/ci/test_acceptance_scenarios.py"
            "::test_every_wire_answers_its_frozen_classes"
        )
        for rule in (
            "INV.SOURCE.SUBSYSTEM-ADDRESS",
            "INV.SOURCE.SUBSYSTEM-DEADLINE-UNAVAILABLE",
        ):
            with self.subTest(rule=rule):
                self.assertEqual(check(rule), corpus_check)
        topology = check("INV.SOURCE.SUBSYSTEM-TOPOLOGY")
        for name in (
            "pointing_at_the_subsystems_folder_answers_only_with_tree",
            "concrete_subsystem_contains_its_root_chain_and_complete_descendant_tree",
            "unregistered_alias_keeps_local_data_without_borrowing_a_registered_tree",
            "root_subsystems_symlink_is_not_followed_for_a_tree_answer",
            "nested_subsystems_symlink_is_not_followed_for_a_tree_answer",
            "subsystem_info_answers_content_and_command_interface_at_once",
            "a_missing_command_interface_is_null_not_an_empty_interface",
        ):
            self.assertIn(name, topology)
    def test_reader_records_retire_with_the_readers_they_governed(self) -> None:
        """Мост читателей снят вместе с ними: правила моста стоят на корпусе.

        Пока мост жил, оба правила ссылались на инвентарь режимов миграции.
        Инвентаря больше нет — как и мостовых читателей, — поэтому оба правила
        переведены в `superseded` и указывают на приёмочный прогон, который
        замораживает ответы канонической поверхности на тех же узлах.
        """
        corpus_check = (
            "tests/ci/test_acceptance_scenarios.py"
            "::test_every_wire_answers_its_frozen_classes"
        )
        for rule in ("INV.SOURCE.READER-MIGRATION", "INV.SOURCE.READER-OUTPUT-PARITY"):
            with self.subTest(rule=rule):
                self.assertEqual(check(rule), corpus_check)
                self.assertIn("status: superseded", record(rule))
        source = (
            REPO_ROOT / "crates" / "unica-coder" / "src" / "application" /
            "tool_contracts.rs"
        ).read_text(encoding="utf-8")
        self.assertNotIn("ReaderMigrationMode", source)
        self.assertNotIn("BRIDGED_SELECTORS", source)

    def test_broad_source_records_name_complete_behavior_checks(self) -> None:
        """Широкая запись называет весь набор проверок, а не одну за всех.

        Раньше здесь стояло имя обёртки, и запись выглядела полной, пока
        обёртка держала список вызовов. Список ведёт запись, поэтому проверять
        надо его состав.
        """
        expected_names = {
            "INV.SOURCE.LOGICAL-IDENTITY": {
                "source_target_profile_emits_canonical_english_kind_tokens",
                "source_target_profile_normalizes_only_registered_exact_russian_kind_aliases",
                "source_target_profile_preserves_application_name_case",
                "source_target_and_resolved_target_serialize_only_logical_identity",
            },
            "INV.SOURCE.WRITE-TARGET-KIND": {
                "platform_xml_target_kind_policy_table_is_closed",
                "platform_xml_source_root_handle_revalidates_without_widening",
                "platform_xml_source_target_revalidation_rejects_changed_descriptor_identity",
            },
            "INV.SOURCE.TAIL-INSERT": {
                "code_patch_tail_insert_public_contract_is_closed",
                "code_patch_without_a_selector_appends_to_the_end_and_proves_the_repeat",
                "code_patch_writes_the_first_body_of_an_empty_or_bom_only_module",
                "code_patch_creates_a_module_file_the_platform_never_exported",
                "code_patch_refuses_a_module_role_the_metadata_kind_never_owns",
            },
            "INV.SOURCE.ROOT-READINESS": {
                "project_health_workspace_root_rejection_suppresses_source_derived_git_facts",
            },
        }
        for record_id, names in expected_names.items():
            with self.subTest(record_id=record_id):
                named = check(record_id)
                for name in names:
                    self.assertIn(name, named)

    def test_autodetect_check_is_driven_by_the_production_catalog(self) -> None:
        self.assertTrue(
            check("INV.SOURCE.AUTODETECT-CATALOG").endswith(
                "::autodetect_catalog_contract_is_closed_over_production_layouts"
            )
        )

    def test_open_schema_and_tail_checks_are_exact_allowlists(self) -> None:
        self.assertTrue(
            check("INV.SOURCE.LOGICAL-INPUT").endswith(
                "::logical_only_tool_schemas_match_exact_property_allowlists"
            )
        )
        source = (
            REPO_ROOT / "crates" / "unica-coder" / "src" / "application" /
            "tool_contracts.rs"
        ).read_text(encoding="utf-8")
        self.assertIn("public_code_mutator_inventory_is_exact", source)

    def test_no_format_migration_uses_exact_surface_and_behavior(self) -> None:
        named = check("INV.SOURCE.NO-FORMAT-MIGRATION")
        for name in (
            "native_mutation_surface_has_exact_operations_and_schemas",
            "public_platform_xml_mutators_have_closed_pre_side_effect_format_refusal",
            "dcs_edit_blocks_old_external_source_set_via_owner_descriptor",
            "cf_init_public_guard_blocks_newer_existing_post_validation_dependency",
        ):
            self.assertIn(name, named)

    def test_portable_resource_role_and_lfs_readiness_are_aggregated(self) -> None:
        named = check("INV.SOURCE.PORTABLE-LFS-ADVISORY")
        for name in (
            "project_health_repository_policy_lfs_is_advisory_for_exact_large_binary",
            "lfs_advice_is_informational_and_does_not_close_readiness",
        ):
            self.assertIn(name, named)
        portable = (
            REPO_ROOT / "crates" / "unica-coder" / "tests" / "platform" /
            "project_health.rs"
        ).read_text(encoding="utf-8")
        self.assertIn("project_health_platform_xml_resource_roles_are_exact", portable)


if __name__ == "__main__":
    unittest.main()
