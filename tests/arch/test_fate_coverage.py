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
                    "product",
                    "tests/ci/test_product_contracts.py::"
                    "test_rmcp_module_preserves_legacy_public_exports_only",
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

    def test_live_safety_and_compatibility_guarantees_are_not_retired(self) -> None:
        records = {}
        for path in (REPO_ROOT / "arch").rglob("*.md"):
            props = FATE_GUARD._front_matter_props(path)
            if identifier := props.get("id"):
                records[identifier] = props
        rows = {row.subject: row for row in FATE_GUARD.fate_rows(REPO_ROOT)}
        required = {
            "INV-CACHE-WRITE-FREE-PREVIEW": {
                "INV.CACHE.INDEX-PREVIEW-WRITE-FREE",
                "INV.SAFETY.PREVIEW-BY-DEFAULT",
            },
            "REQ-SAFETY-PREVIEW-BY-DEFAULT": {
                "INV.SAFETY.PREVIEW-BY-DEFAULT",
            },
            "REQ-SAFETY-SECRET-REDACTION": {
                "INV.SAFETY.STREAM-SECRET-REDACTION",
                "INV.SAFETY.RUNTIME-SECRET-REDACTION",
                "INV.SAFETY.CONFIG-ERROR-REDACTION",
            },
            "REQ-COMPAT-ALL-TARGETS-GREEN": {
                "INV.CI.ALL-TARGETS-GREEN",
            },
            "REQ-COMPAT-OLDEST-CLIENT-LOAD": {
                "INV.PKG.OLDEST-CLIENT-LOAD",
            },
        }
        exact_checks = {
            "INV.SAFETY.PREVIEW-BY-DEFAULT": (
                "product",
                "crates/unica-coder/src/infrastructure/application_ports.rs::"
                "public_preview_strategies_are_real_and_recursively_write_free",
            ),
            "INV.SAFETY.STREAM-SECRET-REDACTION": (
                "product",
                "crates/unica-coder/src/infrastructure/internal_adapters.rs::"
                "production_secret_redaction_surfaces_are_closed",
            ),
            "INV.SAFETY.RUNTIME-SECRET-REDACTION": (
                "product",
                "crates/unica-coder/src/infrastructure/internal_adapters.rs::"
                "production_secret_redaction_surfaces_are_closed",
            ),
            "INV.SAFETY.CONFIG-ERROR-REDACTION": (
                "product",
                "crates/unica-coder/src/infrastructure/internal_adapters.rs::"
                "production_secret_redaction_surfaces_are_closed",
            ),
            "INV.CI.ALL-TARGETS-GREEN": (
                "process",
                "tests/ci/test_unica_workflow.py::"
                "test_every_supported_target_must_pass_before_publication",
            ),
            "INV.PKG.OLDEST-CLIENT-LOAD": (
                "product",
                "tests/ci/test_product_contracts.py::"
                "test_release_gate_pins_the_oldest_supported_client",
            ),
        }

        errors = []
        for subject, successors in required.items():
            row = rows[subject]
            if row.fate not in {"carried", "superseded"}:
                errors.append(f"{subject}: live guarantee is {row.fate!r}")
            if set(row.successors) != successors:
                errors.append(
                    f"{subject}: successors {set(row.successors)!r}, "
                    f"expected {successors!r}"
                )
        for identifier, (governs, check) in exact_checks.items():
            record = records.get(identifier)
            if record is None:
                errors.append(f"missing {identifier}")
                continue
            if record.get("governs") != governs:
                errors.append(
                    f"{identifier}: governs {record.get('governs')!r}, "
                    f"expected {governs!r}"
                )
            if record.get("check") != check:
                errors.append(
                    f"{identifier}: check {record.get('check')!r}, "
                    f"expected {check!r}"
                )

        self.assertEqual(errors, [])

    def test_sdk_module_export_boundary_preserves_its_product_api(self) -> None:
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
        self.assertEqual(decision.get("governs"), "product")
        self.assertEqual(
            decision.get("realized"),
            "tests/ci/test_product_contracts.py::"
            "test_rmcp_module_preserves_legacy_public_exports_only",
        )
        self.assertEqual(
            decision.get("establishes"), "[INV.WIRE.SDK-MODULE-EXPORTS]"
        )
        self.assertNotIn(
            "INV.WIRE.SDK-MODULE-EXPORTS",
            records["DEC.2026-08-18.CARRIED-RULES"]["establishes"],
        )

    def test_task6_round2_evidence_runs_real_paths_and_owns_source_apply_removal(self) -> None:
        records = {}
        for path in (REPO_ROOT / "arch").rglob("*.md"):
            props = FATE_GUARD._front_matter_props(path)
            if identifier := props.get("id"):
                records[identifier] = props
        rows = {row.subject: row for row in FATE_GUARD.fate_rows(REPO_ROOT)}

        exact_checks = {
            "INV.SAFETY.PREVIEW-BY-DEFAULT": (
                "crates/unica-coder/src/infrastructure/application_ports.rs::"
                "public_preview_strategies_are_real_and_recursively_write_free"
            ),
            "INV.TOKEN.CACHE-IMPACT-IN-RESULT": (
                "crates/unica-coder/src/infrastructure/daemon/server.rs::"
                "canonical_object_remove_reports_typed_cache_impact_in_preview_and_publication"
            ),
            "INV.SAFETY.SUPPORT-GUARD-COVERAGE": (
                "crates/unica-coder/src/infrastructure/support_guard.rs::"
                "public_support_guard_resolver_matrix_runs_real_handlers"
            ),
            "INV.SAFETY.SUPPORT-GUARD-PARITY": (
                "crates/unica-coder/src/infrastructure/support_guard.rs::"
                "public_support_guard_resolver_matrix_runs_real_handlers"
            ),
            "INV.SAFETY.RUNTIME-SECRET-REDACTION": (
                "crates/unica-coder/src/infrastructure/internal_adapters.rs::"
                "production_secret_redaction_surfaces_are_closed"
            ),
            "INV.SAFETY.STREAM-SECRET-REDACTION": (
                "crates/unica-coder/src/infrastructure/internal_adapters.rs::"
                "production_secret_redaction_surfaces_are_closed"
            ),
            "INV.SAFETY.CONFIG-ERROR-REDACTION": (
                "crates/unica-coder/src/infrastructure/internal_adapters.rs::"
                "production_secret_redaction_surfaces_are_closed"
            ),
            "INV.PERF.SERVICE-OPERATION-DEADLINE": (
                "crates/unica-coder/src/infrastructure/workspace_services.rs::"
                "service_request_kind_deadline_matrix_is_exhaustive"
            ),
            "INV.REL.ASSESSMENT-PIN": (
                "tests/ci/test_release_assessment.py::"
                "test_non_default_bsp_ref_is_recorded_in_actual_report"
            ),
            "INV.SURFACE.SOURCE-TOOL-SPECS": (
                "crates/unica-coder/src/application/mod.rs::"
                "source_resource_tools_are_read_only_and_have_no_cache_or_event_effects"
            ),
        }
        self.assertEqual(
            {identifier: records.get(identifier, {}).get("check") for identifier in exact_checks},
            exact_checks,
        )

        removal = records.get("DEC.2026-08-21.SOURCE-READ-ONLY-SURFACE", {})
        self.assertEqual(removal.get("status"), "active")
        self.assertEqual(removal.get("governs"), "product")
        self.assertEqual(
            removal.get("realized"),
            exact_checks["INV.SURFACE.SOURCE-TOOL-SPECS"],
        )
        self.assertEqual(removal.get("establishes"), "[INV.SURFACE.SOURCE-TOOL-SPECS]")
        self.assertIn(
            "INV.SURFACE.SOURCE-TOOL-SPECS",
            rows["REQ-PERF-SOURCE-BOUNDS"].successors,
        )

    def test_reviewed_high_risk_fates_name_semantically_complete_evidence(self) -> None:
        records = {}
        for path in (REPO_ROOT / "arch").rglob("*.md"):
            props = FATE_GUARD._front_matter_props(path)
            if identifier := props.get("id"):
                records[identifier] = props
        rows = {row.subject: row for row in FATE_GUARD.fate_rows(REPO_ROOT)}

        expected_successors = {
            "INV-PRODUCT-PACKAGE-PARITY": {
                "INV.PKG.PACKAGED-PUBLIC-SURFACE",
            },
            "INV-MCP-RUNTIME-RECEIPT": {
                "INV.RUNTIME.EXECUTE-RECEIPT",
                "INV.RUNTIME.RISK-CLASSIFICATION",
                "INV.RUNTIME.PREVIEW-NONEXECUTING",
                "INV.RUNTIME.NO-REFUSAL-FALLBACK",
            },
            "INV-CACHE-REPORTED-EFFECTS": {
                "INV.CACHE.MUTATION-EVENT-COVERAGE",
                "INV.CACHE.EVENT-IMPACT-CLOSED",
                "INV.CACHE.REPORTED-EFFECTS",
            },
            "INV-SOURCE-OBSERVED-EOL": {
                "INV.SOURCE.OBSERVED-EOL-PROFILE",
                "INV.SOURCE.CODE-PATCH-EOL",
            },
            "INV-PKG-VERIFIED-ATOMIC-INSTALL": {
                "INV.PKG.VERIFIED-ATOMIC-INSTALL",
            },
            "INV-PLATFORM-NO-ORPHAN-PROCESSES": {
                "INV.PLATFORM.PROCESS-TREE-LIFECYCLE",
            },
            "INV-DOC-REAL-CHECKS": {
                "INV.REGISTRY.CHECK-EXISTS",
            },
            "REQ-COMPAT-FORMAT-PROFILE": {
                "INV.PRODUCT.FULL-DUMP-PROFILE",
                "INV.SOURCE.WRITABLE-PROFILE",
                "DEC.2026-08-21.PLATFORM-XML-PROFILE",
            },
            "REQ-REL-COLD-INSTALL-BUDGET": {
                "INV.PKG.COLD-INSTALL-STARTUP-BUDGET",
            },
            "REQ-REL-NO-SILENT-STALL": {
                "INV.CI.LINEAR-IDEMPOTENT-PUBLICATION",
            },
            "REQ-TOKEN-NO-EXTRA-ROUNDTRIP": {
                "INV.CACHE.MUTATION-EVENT-COVERAGE",
                "INV.CACHE.EVENT-IMPACT-CLOSED",
                "INV.CACHE.REPORTED-EFFECTS",
                "INV.TOKEN.CACHE-IMPACT-IN-RESULT",
            },
            "INV-MCP-SURFACE-SYNC": {
                "CTR.WIRE.TOOL-SURFACE",
                "INV.REGISTRY.CHECK-EXISTS",
                "INV.REGISTRY.RECIPROCAL-OWNERSHIP",
                "INV.SURFACE.CHANGESET-COHERENCE",
                "INV.SURFACE.PARITY-HARNESS-COVERAGE",
            },
            "INV-MCP-TYPED-RESULT": {
                "INV.APP.CODE-DEFINITION-READINESS",
                "INV.SURFACE.RESULT-CONTRACTS-MATCH-REVIEW",
                "INV.WIRE.PREVIEW-IS-MUTATION-ONLY",
                "INV.WIRE.TYPED-READ-FINALIZER",
            },
        }
        for subject, successors in expected_successors.items():
            with self.subTest(subject=subject):
                self.assertEqual(set(rows[subject].successors), successors)

        expected_retirements = {
            "INV-SOURCE-ROLE-ALLOWLIST": (
                "behavior-removed: DEC.2026-08-21.SOURCE-READ-ONLY-SURFACE"
            ),
            "acceptance/format-profile-8-3-27.md": "historical-only",
            "acceptance/logical-source-addressing-and-resource-access.md": (
                "historical-only"
            ),
        }
        for subject, reason in expected_retirements.items():
            with self.subTest(subject=subject):
                self.assertEqual(rows[subject].fate, "retired")
                self.assertEqual(rows[subject].successors, ())
                self.assertEqual(rows[subject].reason, reason)

        exact_checks = {
            "INV.PKG.PACKAGED-PUBLIC-SURFACE": (
                "crates/unica-bootstrap/tests/platform/verification_contract.rs::"
                "verify_requires_each_lifecycle_to_expose_each_public_tool"
            ),
            "INV.RUNTIME.EXECUTE-RECEIPT": (
                "crates/unica-coder/src/application/mod.rs::"
                "runtime_execute_terminal_result_is_returned_in_original_call"
            ),
            "INV.RUNTIME.RISK-CLASSIFICATION": (
                "crates/unica-coder/src/application/runtime_admission.rs::"
                "runtime_risk_classification_is_closed"
            ),
            "INV.RUNTIME.NO-REFUSAL-FALLBACK": (
                "tests/ci/test_unica_skills.py::"
                "test_shipped_guidance_never_routes_runtime_refusal_through_fallbacks"
            ),
            "INV.RUNTIME.PREVIEW-NONEXECUTING": (
                "crates/unica-coder/src/application/mod.rs::"
                "a_preview_does_not_fetch_an_engine_it_will_not_run"
            ),
            "INV.CACHE.MUTATION-EVENT-COVERAGE": (
                "crates/unica-coder/src/application/mod.rs::"
                "mutating_tools_have_typed_cache_event_or_explicit_non_cache_effect"
            ),
            "INV.CACHE.REPORTED-EFFECTS": (
                "crates/unica-coder/src/infrastructure/daemon/server.rs::"
                "canonical_object_remove_reports_typed_cache_impact_in_preview_and_publication"
            ),
            "INV.CACHE.EVENT-IMPACT-CLOSED": (
                "crates/unica-coder/src/domain/cache.rs::"
                "typed_event_cache_impact_catalog_is_closed"
            ),
            "INV.SOURCE.OBSERVED-EOL-PROFILE": (
                "crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::"
                "observed_line_ending_profile_is_closed"
            ),
            "INV.SOURCE.CODE-PATCH-EOL": (
                "crates/unica-coder/src/infrastructure/native_operations/code.rs::"
                "code_patch_observed_eol_policy_is_closed"
            ),
            "INV.PKG.VERIFIED-ATOMIC-INSTALL": (
                "crates/unica-bootstrap/tests/runtime_install.rs::"
                "verified_install_publishes_exact_closure_atomically"
            ),
            "INV.PLATFORM.PROCESS-TREE-LIFECYCLE": (
                "crates/unica-coder/src/infrastructure/platform/process.rs::"
                "managed_process_tree_lifecycle_is_bounded"
            ),
            "INV.PKG.COLD-INSTALL-STARTUP-BUDGET": (
                "tests/ci/test_package_unica_plugin.py::"
                "test_packaged_mcp_declares_its_own_cold_install_startup_budget"
            ),
            "INV.CI.LINEAR-IDEMPOTENT-PUBLICATION": (
                "tests/ci/test_unica_workflow.py::"
                "test_publication_is_one_linear_pass_ordered_by_needs"
            ),
            "INV.TOKEN.CACHE-IMPACT-IN-RESULT": (
                "crates/unica-coder/src/infrastructure/daemon/server.rs::"
                "canonical_object_remove_reports_typed_cache_impact_in_preview_and_publication"
            ),
            "INV.SURFACE.CHANGESET-COHERENCE": (
                "tests/arch/test_product_immutability.py::"
                "test_surface_ledger_change_without_new_product_ground_is_caught"
            ),
            "INV.SURFACE.PARITY-HARNESS-COVERAGE": (
                "tests/ci/test_unica_mcp_script_parity.py::"
                "test_every_in_scope_tool_has_a_parity_scenario"
            ),
            "INV.WIRE.TYPED-READ-FINALIZER": (
                "crates/unica-coder/src/application/mod.rs::"
                "typed_read_result_contract_is_closed"
            ),
            "INV.APP.CODE-DEFINITION-READINESS": (
                "crates/unica-coder/src/infrastructure/rlm_navigation.rs::"
                "definition_readiness_matrix_never_reports_false_typed_success"
            ),
        }
        for identifier, check in exact_checks.items():
            with self.subTest(identifier=identifier):
                self.assertEqual(records.get(identifier, {}).get("check"), check)

    def test_narrowed_v1_claims_are_explicit_and_product_owned(self) -> None:
        records = {}
        bodies = {}
        for path in (REPO_ROOT / "arch").rglob("*.md"):
            props = FATE_GUARD._front_matter_props(path)
            if identifier := props.get("id"):
                records[identifier] = props
                bodies[identifier] = path.read_text(encoding="utf-8")

        narrowing = "DEC.2026-08-22.EVIDENCE-BOUNDED-PRESERVATION"
        self.assertEqual(records.get(narrowing, {}).get("governs"), "product")
        self.assertEqual(
            records.get(narrowing, {}).get("establishes"),
            "[INV.PKG.PACKAGED-PUBLIC-SURFACE, INV.TOKEN.CACHE-IMPACT-IN-RESULT]",
        )
        # DEC.2026-09-03.V0-13-LEGACY-BATCH-2 re-established the rule when
        # `unica.meta.remove` left the registry; the narrowing decision still
        # lists it among what it once established.
        self.assertEqual(
            records["INV.TOKEN.CACHE-IMPACT-IN-RESULT"].get("decision"),
            "DEC.2026-09-03.V0-13-LEGACY-BATCH-2",
        )
        self.assertEqual(
            records["INV.TOKEN.CACHE-IMPACT-IN-RESULT"].get("governs"), "product"
        )

        cutover = "DEC.2026-08-31.V0-13-SURFACE-FIRST-CUTOVER"
        self.assertEqual(records[cutover].get("governs"), "product")
        self.assertEqual(
            records["INV.PKG.PACKAGED-PUBLIC-SURFACE"].get("decision"), cutover
        )
        self.assertEqual(
            records["INV.PERF.BOOTSTRAP-VERIFY-LIFECYCLES"].get("decision"),
            cutover,
        )

        atomic = bodies["INV.PKG.VERIFIED-ATOMIC-INSTALL"].lower()
        self.assertNotIn("размер", atomic)
        self.assertNotIn("режим", atomic)
        artifact_decision = bodies["DEC.2026-08-19.ARTIFACT-VERSIONED-CACHE"].lower()
        self.assertIn("не хранит размер", artifact_decision)
        self.assertIn("не перепроверяет режим", artifact_decision)

        format_decision = bodies["DEC.2026-08-21.PLATFORM-XML-PROFILE"].lower()
        self.assertIn("матрица отклонений", format_decision)
        self.assertIn("историч", format_decision)

        packaged = bodies["INV.PKG.PACKAGED-PUBLIC-SURFACE"]
        self.assertIn("initialize", packaged)
        self.assertIn("tools/list", packaged)
        for required_tool in (
            "unica.view",
            "unica.apply",
            "unica.find",
            "unica.search",
            "unica.check",
            "unica.diff",
            "unica.run",
            "unica.docs",
            "unica.task.get",
            "unica.task.result",
            "unica.task.cancel",
        ):
            with self.subTest(required_tool=required_tool):
                self.assertIn(required_tool, packaged)
        self.assertNotIn("канонический набор", packaged.lower())
        self.assertNotIn("схем", packaged.lower())

        for identifier in (
            "DEC.2026-08-22.LINEAR-PUBLICATION",
            "INV.CI.LINEAR-IDEMPOTENT-PUBLICATION",
        ):
            with self.subTest(identifier=identifier):
                self.assertEqual(records[identifier].get("governs"), "product")

    def test_safety_claims_are_owned_by_an_evidence_boundary_decision(self) -> None:
        records = {}
        bodies = {}
        for path in (REPO_ROOT / "arch").rglob("*.md"):
            props = FATE_GUARD._front_matter_props(path)
            if identifier := props.get("id"):
                records[identifier] = props
                bodies[identifier] = path.read_text(encoding="utf-8")

        decision_id = "DEC.2026-08-22.EVIDENCE-BOUNDED-SAFETY"
        decision = records.get(decision_id, {})
        self.assertEqual(decision.get("status"), "active")
        self.assertEqual(decision.get("governs"), "product")
        self.assertEqual(
            decision.get("establishes"),
            "[INV.SAFETY.PREVIEW-BY-DEFAULT, INV.SAFETY.SUPPORT-GUARD-PARITY]",
        )
        for identifier in (
            "INV.SAFETY.PREVIEW-BY-DEFAULT",
            "INV.SAFETY.SUPPORT-GUARD-PARITY",
        ):
            with self.subTest(identifier=identifier):
                self.assertEqual(records[identifier].get("decision"), decision_id)
                self.assertIn("представитель", bodies[identifier].lower())
                self.assertNotIn("Каждый мутирующий инструмент", bodies[identifier])
                self.assertNotIn("Каждая защищённая нативная мутация", bodies[identifier])

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
