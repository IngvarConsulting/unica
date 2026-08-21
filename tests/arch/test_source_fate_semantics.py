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
    match = re.search(r"^check: (.+)$", record(record_id), re.MULTILINE)
    if match is None:
        raise AssertionError(f"{record_id} has no check")
    return match.group(1)


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
        self.assertIn(
            "realized: crates/unica-coder/src/infrastructure/format_guard.rs::single_writable_platform_xml_profile_decision_is_fully_realized",
            profile,
        )
        self.assertNotIn("INV.SOURCE.PLATFORM-BEFORE-XSD", profile)
        self.assertIn(
            "realized: crates/unica-coder/src/infrastructure/native_operations.rs::mutation_idempotence_scope_decision_is_fully_realized",
            mutation,
        )

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
        self.assertTrue(
            check("INV.SOURCE.ROLLBACK-DIAGNOSTIC-CLASS").endswith(
                "::fault_injected_rollback_and_cleanup_paths_keep_distinct_diagnostics"
            )
        )

    def test_subsystem_checks_cover_public_schema_and_no_data_failures(self) -> None:
        self.assertTrue(
            check("INV.SOURCE.SUBSYSTEM-ADDRESS").endswith(
                "::public_subsystem_info_registration_address_and_schema_contract_is_complete"
            )
        )
        self.assertTrue(
            check("INV.SOURCE.SUBSYSTEM-DEADLINE-UNAVAILABLE").endswith(
                "::public_subsystem_info_deadline_returns_no_data"
            )
        )
        self.assertTrue(
            check("INV.SOURCE.SUBSYSTEM-TOPOLOGY").endswith(
                "::public_subsystem_projection_and_mode_absence_contract_is_complete"
            )
        )

    def test_reader_records_share_one_authoritative_migration_inventory(self) -> None:
        expected = (
            "crates/unica-coder/src/application/tool_contracts.rs"
            "::subject_reader_migration_inventory_is_complete"
        )
        self.assertEqual(check("INV.SOURCE.READER-MIGRATION"), expected)
        self.assertIn(
            "authoritative_reader_migration_inventory",
            record("INV.SOURCE.READER-OUTPUT-PARITY"),
        )
        source = (
            REPO_ROOT / "crates" / "unica-coder" / "src" / "application" /
            "tool_contracts.rs"
        ).read_text(encoding="utf-8")
        owner = source.index("pub(crate) enum ReaderMigrationMode")
        self.assertNotIn("#[cfg(test)]", source[max(0, owner - 80):owner])

    def test_broad_source_records_name_complete_behavior_checks(self) -> None:
        expected_suffixes = {
            "INV.SOURCE.LOGICAL-IDENTITY":
                "::logical_target_identity_contract_is_complete",
            "INV.SOURCE.WRITE-TARGET-KIND":
                "::write_target_kind_and_revalidation_contract_is_complete",
            "INV.SOURCE.TAIL-INSERT":
                "::tail_insert_public_and_write_contract_is_complete",
            "INV.SOURCE.ROOT-READINESS":
                "::project_status_workspace_root_rejection_preserves_the_entire_tree",
        }
        for record_id, suffix in expected_suffixes.items():
            with self.subTest(record_id=record_id):
                self.assertTrue(check(record_id).endswith(suffix))

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
        self.assertTrue(
            check("INV.SOURCE.NO-FORMAT-MIGRATION").endswith(
                "::native_mutation_surface_and_format_refusal_are_exact"
            )
        )

    def test_portable_resource_role_and_lfs_readiness_are_aggregated(self) -> None:
        self.assertTrue(
            check("INV.SOURCE.PORTABLE-LFS-ADVISORY").endswith(
                "::portable_lfs_advice_and_readiness_contract_is_complete"
            )
        )
        portable = (
            REPO_ROOT / "crates" / "unica-coder" / "tests" / "platform" /
            "project_health.rs"
        ).read_text(encoding="utf-8")
        self.assertIn("project_health_platform_xml_resource_roles_are_exact", portable)


if __name__ == "__main__":
    unittest.main()
