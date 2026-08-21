"""Contract test for the architecture-v1 fate ledger."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
GUARD = REPO_ROOT / "scripts" / "arch" / "fate.py"
SPEC = importlib.util.spec_from_file_location("arch_fate", GUARD)
FATE_GUARD = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = FATE_GUARD
SPEC.loader.exec_module(FATE_GUARD)


COMPLETE_FATE = """# Fate

| Subject | Fate | Successor | Reason |
| --- | --- | --- | --- |
| `ADR-0001` | `retired` | — | `historical-only` |
| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |
| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |
| `acceptance/runtime.md` | `carried` | `CTR.WIRE.RUNTIME` | — |
"""


def carried_successor_side_errors(
    row: FATE_GUARD.FateRow,
    records: dict[str, dict[str, str]],
    expected: str,
) -> list[str]:
    if not row.successors:
        return [f"{row.subject}: expected at least one carried successor"]
    errors = []
    for successor in row.successors:
        record = records.get(successor)
        if record is None:
            errors.append(f"{row.subject}: successor {successor} does not resolve")
            continue
        actual = record.get("governs")
        if actual != expected:
            errors.append(
                f"{row.subject}: {successor} governs {actual!r}, expected {expected!r}"
            )
    return errors


class Fixture:
    def __init__(self, stack: tempfile.TemporaryDirectory[str]) -> None:
        self.root = Path(stack.name)
        archive = self.root / "docs" / "arch-v1"
        (archive / "decisions").mkdir(parents=True)
        (archive / "architecture").mkdir()
        (archive / "acceptance").mkdir()
        (archive / "decisions" / "0001-one.md").write_text("# ADR-0001\n", encoding="utf-8")
        (archive / "architecture" / "invariants.md").write_text(
            """### INV-APP-BOUNDARY — Boundary

- **Rule:** The generic application boundary remains independent of tool names.
- **Check:** `ci-test` — `tests/checks.py::test_boundary`
""",
            encoding="utf-8",
        )
        (archive / "architecture" / "quality-requirements.md").write_text(
            "### REQ-PERF-DEADLINE — Deadline\n", encoding="utf-8"
        )
        (archive / "acceptance" / "runtime.md").write_text("# Runtime\n", encoding="utf-8")
        (archive / "FATE.md").write_text(COMPLETE_FATE, encoding="utf-8")

        (self.root / "arch" / "invariants").mkdir(parents=True)
        (self.root / "arch" / "contracts").mkdir()
        (self.root / "arch" / "decisions").mkdir()
        (self.root / "arch" / "invariants" / "INV.APP.BOUNDARY.md").write_text(
            "---\nid: INV.APP.BOUNDARY\n---\n", encoding="utf-8"
        )
        (self.root / "arch" / "contracts" / "CTR.WIRE.RUNTIME.md").write_text(
            "---\nid: CTR.WIRE.RUNTIME\n---\n", encoding="utf-8"
        )
        (self.root / "tests").mkdir()
        (self.root / "tests" / "checks.py").write_text(
            "def test_boundary():\n    pass\n", encoding="utf-8"
        )

    @property
    def fate(self) -> Path:
        return self.root / "docs" / "arch-v1" / "FATE.md"

    def add_decision(
        self,
        identifier: str,
        *,
        status: str = "active",
        governs: str = "product",
    ) -> None:
        slug = identifier.removeprefix("DEC.").lower().replace(".", "-")
        (self.root / "arch" / "decisions" / f"{slug}.md").write_text(
            f"---\nid: {identifier}\nstatus: {status}\ngoverns: {governs}\n---\n",
            encoding="utf-8",
        )

    def add_v1_rule_with_fate(self, subject: str, reason: str) -> None:
        invariants = self.root / "docs" / "arch-v1" / "architecture" / "invariants.md"
        invariants.write_text(
            invariants.read_text(encoding="utf-8") + f"\n### {subject} — Boundary\n",
            encoding="utf-8",
        )
        self.fate.write_text(
            self.fate.read_text(encoding="utf-8")
            + f"| `{subject}` | `retired` | — | `{reason}` |\n",
            encoding="utf-8",
        )

    def run(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(GUARD), "--root", str(self.root)],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )


class FateCoverageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = tempfile.TemporaryDirectory()
        self.addCleanup(self.stack.cleanup)
        self.fixture = Fixture(self.stack)

    def test_a_complete_fate_ledger_passes(self) -> None:
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_missing_v1_subject_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |\n",
                "",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REQ-PERF-DEADLINE", result.stderr)

    def test_a_duplicate_v1_subject_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE
            + "| `ADR-0001` | `retired` | — | `historical-only` |\n",
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ADR-0001", result.stderr)
        self.assertIn("duplicate", result.stderr.lower())

    def test_an_unknown_fate_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace("`retired`", "`alive`", 1), encoding="utf-8"
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("alive", result.stderr)

    def test_an_unresolved_successor_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace("INV.APP.BOUNDARY", "INV.APP.MISSING", 1),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV.APP.MISSING", result.stderr)

    def test_a_retired_subject_rejects_raw_successor_text(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `ADR-0001` | `retired` | — | `historical-only` |",
                "| `ADR-0001` | `retired` | legacy-replacement | `historical-only` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ADR-0001", result.stderr)
        self.assertIn("legacy-replacement", result.stderr)

    def test_a_successor_cell_rejects_text_left_after_a_v2_id(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` legacy | — |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV-APP-BOUNDARY", result.stderr)
        self.assertIn("legacy", result.stderr)

    def test_multiple_successors_accept_documented_separators(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |",
                "| `REQ-PERF-DEADLINE` | `superseded` | "
                "`INV.APP.BOUNDARY`,<br>`CTR.WIRE.RUNTIME` | — |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_missing_retirement_reason_is_rejected(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace("`historical-only`", "—", 1), encoding="utf-8"
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ADR-0001", result.stderr)
        self.assertIn("reason", result.stderr.lower())

    def test_a_carried_subject_cannot_name_a_retirement_reason(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | `historical-only` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV-APP-BOUNDARY", result.stderr)
        self.assertIn("reason", result.stderr.lower())

    def test_a_rule_cannot_be_called_historical_only(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `retired` | — | `historical-only` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV-APP-BOUNDARY", result.stderr)
        self.assertIn("historical-only", result.stderr)

    def test_a_live_check_cannot_be_called_removed(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `retired` | — | `check-removed` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV-APP-BOUNDARY", result.stderr)
        self.assertIn("tests/checks.py::test_boundary", result.stderr)

    def test_check_removed_requires_an_old_check(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |",
                "| `REQ-PERF-DEADLINE` | `retired` | — | `check-removed` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REQ-PERF-DEADLINE", result.stderr)
        self.assertIn("old check", result.stderr.lower())

    def test_a_missing_named_check_allows_check_removed(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `retired` | — | `check-removed` |",
            ),
            encoding="utf-8",
        )
        (self.fixture.root / "tests" / "checks.py").write_text(
            "def another_check():\n    pass\n", encoding="utf-8"
        )
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_generic_rule_cannot_claim_tool_surface_retirement(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `retired` | — | `tool-surface-bound` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV-APP-BOUNDARY", result.stderr)
        self.assertIn("unica.*", result.stderr)

    def test_a_literal_tool_name_allows_tool_surface_retirement(self) -> None:
        invariants = (
            self.fixture.root / "docs" / "arch-v1" / "architecture" / "invariants.md"
        )
        invariants.write_text(
            invariants.read_text(encoding="utf-8").replace(
                "generic application boundary", "public `unica.meta.info` boundary"
            ),
            encoding="utf-8",
        )
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                "| `INV-APP-BOUNDARY` | `retired` | — | `tool-surface-bound` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_behavior_removed_requires_a_resolving_decision(self) -> None:
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |",
                "| `REQ-PERF-DEADLINE` | `retired` | — | `behavior-removed: DEC.2026-08-21.MISSING` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("DEC.2026-08-21.MISSING", result.stderr)
        self.assertIn("does not resolve", result.stderr)

    def test_behavior_removed_requires_an_active_decision(self) -> None:
        decision = "DEC.2026-08-21.REMOVAL"
        self.fixture.add_decision(decision, status="planned")
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |",
                f"| `REQ-PERF-DEADLINE` | `retired` | — | `behavior-removed: {decision}` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(decision, result.stderr)
        self.assertIn("planned", result.stderr)

    def test_an_active_decision_allows_behavior_removed(self) -> None:
        decision = "DEC.2026-08-21.REMOVAL"
        self.fixture.add_decision(decision)
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |",
                f"| `REQ-PERF-DEADLINE` | `retired` | — | `behavior-removed: {decision}` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_product_behavior_cannot_be_removed_by_a_process_decision(self) -> None:
        decision = "DEC.2026-08-21.PROCESS-REMOVAL"
        self.fixture.add_decision(decision, governs="process")
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `REQ-PERF-DEADLINE` | `superseded` | `INV.APP.BOUNDARY` | — |",
                f"| `REQ-PERF-DEADLINE` | `retired` | — | `behavior-removed: {decision}` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REQ-PERF-DEADLINE", result.stderr)
        self.assertIn("expected 'product'", result.stderr)
        self.assertIn("got 'process'", result.stderr)

    def test_unclassified_behavior_cannot_cite_a_removal_decision(self) -> None:
        decision = "DEC.2026-08-21.REMOVAL"
        self.fixture.add_decision(decision)
        self.fixture.fate.write_text(
            COMPLETE_FATE.replace(
                "| `INV-APP-BOUNDARY` | `carried` | `INV.APP.BOUNDARY` | — |",
                f"| `INV-APP-BOUNDARY` | `retired` | — | `behavior-removed: {decision}` |",
            ),
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INV-APP-BOUNDARY", result.stderr)
        self.assertIn("cannot classify", result.stderr)

    def test_process_behavior_can_be_removed_by_a_process_decision(self) -> None:
        decision = "DEC.2026-08-21.PROCESS-REMOVAL"
        self.fixture.add_decision(decision, governs="process")
        requirements = (
            self.fixture.root
            / "docs"
            / "arch-v1"
            / "architecture"
            / "quality-requirements.md"
        )
        requirements.write_text(
            requirements.read_text(encoding="utf-8")
            + "\n### REQ-MAINT-BOUNDARY — Boundary\n",
            encoding="utf-8",
        )
        self.fixture.fate.write_text(
            COMPLETE_FATE
            + f"| `REQ-MAINT-BOUNDARY` | `retired` | — | "
            f"`behavior-removed: {decision}` |\n",
            encoding="utf-8",
        )
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_code_provider_boundary_accepts_a_product_removal_decision(self) -> None:
        decision = "DEC.2026-08-21.PRODUCT-REMOVAL"
        self.fixture.add_decision(decision, governs="product")
        self.fixture.add_v1_rule_with_fate(
            "INV-APP-CODE-PROVIDER-BOUNDARY", f"behavior-removed: {decision}"
        )
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_code_provider_boundary_rejects_a_process_removal_decision(self) -> None:
        decision = "DEC.2026-08-21.PROCESS-REMOVAL"
        self.fixture.add_decision(decision, governs="process")
        self.fixture.add_v1_rule_with_fate(
            "INV-APP-CODE-PROVIDER-BOUNDARY", f"behavior-removed: {decision}"
        )
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected 'product'", result.stderr)
        self.assertIn("got 'process'", result.stderr)

    def test_carried_classified_subjects_keep_their_successor_side(self) -> None:
        records = {}
        for path in (REPO_ROOT / "arch").rglob("*.md"):
            props = FATE_GUARD._front_matter_props(path)
            if identifier := props.get("id"):
                records[identifier] = props

        for row in FATE_GUARD.fate_rows(REPO_ROOT):
            expected = FATE_GUARD._legacy_governs(row.subject)
            if row.fate != "carried" or expected is None:
                continue
            with self.subTest(subject=row.subject):
                self.assertEqual(carried_successor_side_errors(row, records, expected), [])

    def test_carried_side_consistency_accepts_two_product_successors(self) -> None:
        row = FATE_GUARD.FateRow(
            subject="REQ-PERF-DEADLINE",
            fate="carried",
            successor_cell="`INV.APP.ONE`, `INV.APP.TWO`",
            successors=("INV.APP.ONE", "INV.APP.TWO"),
            reason="—",
        )
        records = {
            "INV.APP.ONE": {"governs": "product"},
            "INV.APP.TWO": {"governs": "product"},
        }
        self.assertEqual(carried_successor_side_errors(row, records, "product"), [])

    def test_carried_side_consistency_rejects_a_mixed_successor_set(self) -> None:
        row = FATE_GUARD.FateRow(
            subject="REQ-PERF-DEADLINE",
            fate="carried",
            successor_cell="`INV.APP.ONE`, `INV.APP.TWO`",
            successors=("INV.APP.ONE", "INV.APP.TWO"),
            reason="—",
        )
        records = {
            "INV.APP.ONE": {"governs": "product"},
            "INV.APP.TWO": {"governs": "process"},
        }
        self.assertEqual(
            carried_successor_side_errors(row, records, "product"),
            [
                "REQ-PERF-DEADLINE: INV.APP.TWO governs 'process', "
                "expected 'product'"
            ],
        )

    def test_mandatory_mcp_fates_name_exact_architecture_evidence(self) -> None:
        records = {}
        for path in (REPO_ROOT / "arch").rglob("*.md"):
            props = FATE_GUARD._front_matter_props(path)
            if identifier := props.get("id"):
                records[identifier] = props
        rows = {row.subject: row for row in FATE_GUARD.fate_rows(REPO_ROOT)}
        expected = {
            "INV-MCP-DATA-DRIVEN-SCHEMA": {
                "INV.WIRE.DATA-DRIVEN-TOOL-LIST": (
                    "process",
                    "crates/unica-coder/src/interfaces/mcp.rs::"
                    "application_registry_owns_tool_names_descriptions_and_wire_schemas",
                ),
                "INV.SURFACE.NO-RAW-ADAPTER-ARGS": (
                    "product",
                    "crates/unica-coder/src/interfaces/mcp.rs::"
                    "no_public_tool_schema_exposes_raw_adapter_args",
                ),
            },
            "INV-MCP-SDK-TRANSPORT": {
                "INV.WIRE.SDK-DEPENDENCY": (
                    "process",
                    "tests/ci/test_product_contracts.py::"
                    "test_rmcp_dependency_is_owned_by_unica_coder_without_macro_features",
                ),
                "INV.WIRE.SDK-MODULE-EXPORTS": (
                    "process",
                    "tests/ci/test_product_contracts.py::"
                    "test_rmcp_module_exports_only_run_stdio",
                ),
                "INV.WIRE.SDK-SERVER-HANDLER": (
                    "product",
                    "tests/ci/test_product_contracts.py::"
                    "test_unica_coder_production_library_satisfies_rmcp_handler_bound",
                ),
                "INV.WIRE.SDK-TRANSPORT": (
                    "process",
                    "tests/ci/test_product_contracts.py::"
                    "test_rmcp_transport_is_confined_to_mcp_interface",
                ),
                "INV.WIRE.SDK-INITIALIZE": (
                    "product",
                    "crates/unica-coder/src/interfaces/mcp.rs::"
                    "initialize_uses_single_public_server_name_and_negotiates_version",
                ),
                "INV.WIRE.DIRECT-FIRST-LIFECYCLE": (
                    "product",
                    "crates/unica-coder/src/interfaces/mcp.rs::"
                    "modern_direct_first_tools_list_pages_through_the_full_registry",
                ),
            },
            "INV-MCP-VERSION-TIERS": {
                "INV.WIRE.GUARANTEED-VERSIONS": (
                    "product",
                    "crates/unica-bootstrap/tests/platform/verification_contract.rs::"
                    "verify_rejects_discover_without_the_guaranteed_versions",
                ),
                "INV.WIRE.PINNED-FALLBACK-VERSION": (
                    "product",
                    "crates/unica-coder/src/interfaces/mcp.rs::"
                    "legacy_unknown_offer_falls_back_to_pinned_version",
                ),
            },
            "INV-MCP-DEFERRED-READ": {
                "INV.APP.DEFERRED-MANIFEST": (
                    "product",
                    "crates/unica-coder/src/application/mod.rs::"
                    "oversized_typed_read_returns_a_manifest_within_budget",
                ),
                "INV.APP.DEFERRED-READ": (
                    "product",
                    "crates/unica-coder/src/application/mod.rs::"
                    "continuation_slices_byte_stably_without_rereading_the_source",
                ),
            },
        }
        self.assertEqual(
            set(expected),
            {
                "INV-MCP-DATA-DRIVEN-SCHEMA",
                "INV-MCP-SDK-TRANSPORT",
                "INV-MCP-VERSION-TIERS",
                "INV-MCP-DEFERRED-READ",
            },
            "all four mandatory MCP subjects need exact successor evidence",
        )

        errors = []
        for subject, required in expected.items():
            row = rows[subject]
            if set(row.successors) != set(required):
                errors.append(
                    f"{subject}: successors {row.successors!r}, "
                    f"expected {tuple(required)!r}"
                )
            for successor, (governs, check) in required.items():
                record = records.get(successor)
                if record is None:
                    errors.append(f"{subject}: missing {successor}")
                    continue
                if record.get("governs") != governs:
                    errors.append(
                        f"{successor}: governs {record.get('governs')!r}, "
                        f"expected {governs!r}"
                    )
                if record.get("check") != check:
                    errors.append(
                        f"{successor}: check {record.get('check')!r}, "
                        f"expected {check!r}"
                    )

        self.assertEqual(errors, [])

    def test_sdk_module_export_boundary_has_its_new_process_decision(self) -> None:
        records = {}
        for path in (REPO_ROOT / "arch").rglob("*.md"):
            props = FATE_GUARD._front_matter_props(path)
            if identifier := props.get("id"):
                records[identifier] = props

        invariant = records["INV.WIRE.SDK-MODULE-EXPORTS"]
        self.assertEqual(
            invariant.get("decision"),
            "DEC.2026-08-21.SDK-MODULE-EXPORT-BOUNDARY",
        )
        decision = records.get("DEC.2026-08-21.SDK-MODULE-EXPORT-BOUNDARY")
        self.assertIsNotNone(decision)
        assert decision is not None
        self.assertEqual(decision.get("status"), "active")
        self.assertEqual(decision.get("governs"), "process")
        self.assertEqual(
            decision.get("realized"),
            "tests/ci/test_product_contracts.py::"
            "test_rmcp_module_exports_only_run_stdio",
        )
        self.assertEqual(
            decision.get("establishes"), "[INV.WIRE.SDK-MODULE-EXPORTS]"
        )
        self.assertNotIn(
            "INV.WIRE.SDK-MODULE-EXPORTS",
            records["DEC.2026-08-18.CARRIED-RULES"]["establishes"],
        )

    def test_every_v1_subject_has_exactly_one_fate(self) -> None:
        """A moved ADR, rule, requirement, or acceptance contract cannot disappear."""
        result = subprocess.run(
            [sys.executable, str(GUARD), "--root", str(REPO_ROOT)],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
