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
    def test_adr_0016_names_the_complete_profile_successor_set(self) -> None:
        fate = (REPO_ROOT / "docs" / "arch-v1" / "FATE.md").read_text(
            encoding="utf-8"
        )
        row = next(line for line in fate.splitlines() if "`ADR-0016`" in line)
        for successor in (
            "DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE",
            "INV.SOURCE.WRITABLE-PROFILE",
            "INV.SOURCE.OWNER-VERSION-GATE",
            "INV.SOURCE.NO-FORMAT-MIGRATION",
            "INV.SOURCE.PLATFORM-BEFORE-XSD",
        ):
            self.assertIn(successor, row)

    def test_portable_git_and_platform_xml_checks_are_closed_matrices(self) -> None:
        self.assertTrue(
            check("INV.SOURCE.PORTABLE-GIT").endswith(
                "::portable_git_readiness_contract_is_a_closed_positive_and_negative_matrix"
            )
        )
        self.assertTrue(
            check("INV.SOURCE.PLATFORM-XML-ONLY").endswith(
                "::native_platform_xml_source_format_guard_is_closed_over_public_operations"
            )
        )

    def test_mutator_rewrite_and_preimage_checks_use_the_closed_public_inventory(self) -> None:
        self.assertTrue(
            check("INV.SOURCE.IDEMPOTENT-REWRITE").endswith(
                "::public_platform_xml_mutator_idempotence_contract_is_complete"
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


if __name__ == "__main__":
    unittest.main()
