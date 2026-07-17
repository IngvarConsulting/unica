from __future__ import annotations

import importlib.util
import json
import re
import sqlite3
import tempfile
import unittest
from pathlib import Path


def load_contract_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "check-tool-contracts.py"
    spec = importlib.util.spec_from_file_location("check_tool_contracts", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def parse_fenced_json_blocks(markdown: str) -> tuple[list[str], int]:
    blocks: list[str] = []
    block_lines: list[str] = []
    opener_line: int | None = None
    opener_count = 0

    for line_number, line in enumerate(markdown.splitlines(), start=1):
        stripped = line.strip()
        if opener_line is None:
            if stripped.startswith("```json"):
                opener_count += 1
                if stripped != "```json":
                    raise ValueError(f"malformed json fence opener at line {line_number}")
                opener_line = line_number
                block_lines = []
            continue

        if stripped.startswith("```"):
            if stripped != "```":
                raise ValueError(f"malformed json fence closer at line {line_number}")
            blocks.append("\n".join(block_lines))
            opener_line = None
            block_lines = []
            continue

        block_lines.append(line)

    if opener_line is not None:
        raise ValueError(f"unclosed json fence opened at line {opener_line}")
    if len(blocks) != opener_count:
        raise ValueError(f"json fence count mismatch: {opener_count} openers, {len(blocks)} blocks")
    return blocks, opener_count


class ProductContractTests(unittest.TestCase):
    def test_json_fence_parser_rejects_malformed_and_unclosed_fences(self) -> None:
        blocks, opener_count = parse_fenced_json_blocks("before\n```json\n{}\n```\nafter")
        self.assertEqual(blocks, ["{}"])
        self.assertEqual(opener_count, 1)

        with self.assertRaisesRegex(ValueError, "malformed json fence opener"):
            parse_fenced_json_blocks("```json trailing-text\n{}\n```")
        with self.assertRaisesRegex(ValueError, "malformed json fence closer"):
            parse_fenced_json_blocks("```json\n{}\n```json")
        with self.assertRaisesRegex(ValueError, "unclosed json fence"):
            parse_fenced_json_blocks("```json\n{}")

    def test_ai_entrypoints_document_source_of_truth_and_ignored_corpus(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        entrypoint = repo_root / "AGENTS.md"

        text = entrypoint.read_text(encoding="utf-8")

        self.assertIn("code/tests/package metadata > spec > historical plans", text)
        for ignored in ["docs/research", "docs/its", "target", ".build", "dist"]:
            with self.subTest(ignored=ignored):
                self.assertIn(ignored, text)

    def test_readme_describes_checked_in_source_manifest_placeholder(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        readme = (repo_root / "README.md").read_text(encoding="utf-8")

        self.assertIn("checked-in placeholder `third-party/manifest.json`", readme)
        self.assertIn("generated marketplace archives overwrite", readme)

    def test_superpowers_plans_are_marked_historical(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        plan_dir = repo_root / "docs" / "superpowers" / "plans"

        for plan in plan_dir.glob("*.md"):
            with self.subTest(plan=plan.name):
                head = "\n".join(plan.read_text(encoding="utf-8").splitlines()[:8])
                self.assertIn("Historical", head)

    def test_script_backed_skill_exceptions_are_documented_by_adr(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        adr = repo_root / "spec" / "decisions" / "0007-script-backed-utility-skill-exceptions.md"

        text = adr.read_text(encoding="utf-8")

        self.assertIn("web-test", text)
        self.assertIn("img-grid", text)
        self.assertIn("permanent local-tool exception", text)

    def test_project_discovery_architecture_is_synchronized(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        design_path = repo_root / "spec" / "architecture" / "extension-point-discovery.md"
        adr_path = repo_root / "spec" / "decisions" / "0008-project-discovery-and-discovery-receipts.md"

        self.assertTrue(adr_path.exists(), "ADR 0008 must record the accepted discovery architecture")

        design = design_path.read_text(encoding="utf-8")
        historical_plan = (
            repo_root / "docs" / "superpowers" / "plans" / "2026-07-17-project-discovery-receipts.md"
        ).read_text(encoding="utf-8")
        adr = adr_path.read_text(encoding="utf-8")
        spec_readme = (repo_root / "spec" / "README.md").read_text(encoding="utf-8")
        decisions_readme = (repo_root / "spec" / "decisions" / "README.md").read_text(encoding="utf-8")
        invariants = (repo_root / "spec" / "architecture" / "invariants.md").read_text(encoding="utf-8")
        checklist = (repo_root / "spec" / "architecture" / "change-checklist.md").read_text(encoding="utf-8")
        arc42_dir = repo_root / "spec" / "architecture" / "arc42"
        arc42_files = {
            name: " ".join((arc42_dir / name).read_text(encoding="utf-8").split())
            for name in [
                "05-building-block-view.md",
                "06-runtime-view.md",
                "08-cross-cutting-concepts.md",
                "09-architecture-decisions.md",
                "10-quality-requirements.md",
                "11-risks-and-technical-debt.md",
            ]
        }
        normalized_design = " ".join(design.split())
        normalized_adr = " ".join(adr.split())
        normalized_invariants = " ".join(invariants.split())
        normalized_checklist = " ".join(checklist.split())

        self.assertIn("Status: accepted", design)
        self.assertIn("Status: accepted", adr)
        self.assertIn("extension-point-discovery.md", spec_readme)
        self.assertIn("0008-project-discovery-and-discovery-receipts.md", decisions_readme)

        canonical_shape_marker = "Version-1 canonical shapes are:"
        self.assertIn(canonical_shape_marker, design)
        canonical_shape_section = design.split(canonical_shape_marker, 1)[1]
        canonical_shape_lines: list[str] = []
        for line in canonical_shape_section.splitlines():
            if line.startswith("|"):
                canonical_shape_lines.append(line)
            elif canonical_shape_lines:
                break
        self.assertGreaterEqual(len(canonical_shape_lines), 2)
        self.assertEqual(canonical_shape_lines[0], "| `kind` | Canonical `ref` shape |")
        self.assertEqual(canonical_shape_lines[1], "| --- | --- |")
        shape_rows: dict[str, str] = {}
        for line in canonical_shape_lines[2:]:
            cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
            self.assertEqual(len(cells), 2, f"invalid canonical-shape row: {line}")
            kind = cells[0].removeprefix("`").removesuffix("`")
            self.assertNotIn(kind, shape_rows, f"duplicate canonical-shape kind: {kind}")
            shape_rows[kind] = cells[1]

        expected_shape_anchors = {
            "metadata_object": "<ObjectKind>.<ObjectName>",
            "metadata_attribute": "<OwnerRef>.Attribute.<AttributeName>",
            "tabular_section": "<OwnerRef>.TabularSection.<SectionName>",
            "tabular_section_attribute": "<OwnerRef>.TabularSection.<SectionName>.Attribute.<AttributeName>",
            "module": "<OwnerRef>.<ModuleKind>",
            "method": "<ModuleRef>.<MethodName>",
            "form": "<OwnerRef>.Form.<FormName>",
            "form_command": "<FormRef>.Command.<CommandName>",
            "common_command": "CommonCommand.<CommandName>",
            "event_subscription": "EventSubscription.<SubscriptionName>",
            "scheduled_job": "ScheduledJob.<JobName>",
            "http_route": "HTTPService.<ServiceName>.URLTemplate.<TemplateName>.Method.<MethodName>",
            "exchange_plan": "ExchangePlan.<PlanName>",
            "report": "Report.<ReportName>",
            "data_processor": "DataProcessor.<ProcessorName>",
        }
        self.assertEqual(len(shape_rows), 15)
        self.assertEqual(set(shape_rows), set(expected_shape_anchors))
        for kind, shape_anchor in expected_shape_anchors.items():
            with self.subTest(document="canonical shapes", kind=kind):
                self.assertIn(shape_anchor, shape_rows[kind])
        self.assertIn("`CommonModule.<ModuleName>`", shape_rows["module"])
        self.assertRegex(
            canonical_shape_section,
            r"A `CommonModule` is a\s+self-owned `module` with canonical ref\s+"
            r"`CommonModule\.<ModuleName>`; it is not\s+duplicated as a `metadata_object`",
        )

        reserved_clause = re.search(
            r"reserved literals such as\s+(?P<literals>.*?)\s+are\s+exact\s+ASCII tokens",
            canonical_shape_section,
            flags=re.DOTALL,
        )
        self.assertIsNotNone(reserved_clause)
        reserved_literals = set(re.findall(r"`([^`]+)`", reserved_clause.group("literals")))
        self.assertEqual(
            reserved_literals,
            {"Attribute", "TabularSection", "Form", "Command", "FormModule", "URLTemplate", "Method"},
        )

        adr_headings = [
            "### Typed discovery boundary",
            "### Source snapshots and atomic grants",
            "### Lease, mutation, and rolling revision",
            "### Guard and rollout",
            "### Shadow observations",
            "### Version 1 proof boundary",
        ]
        adr_anchors = [
            "`unica.project.discover`",
            "MetadataCatalogPort",
            "CodeSearchPort",
            "DefinitionPort",
            "CallGraphPort",
            "FormInspectionPort",
            "SupportStatePort",
            "`workspaceEpoch`",
            "`supported`",
            "`contradicted`",
            "`unknown`",
            "`not_required`",
            "`advisory_only`",
            "`enforceable`",
            "`off`",
            "`observe`",
            "`warn`",
            "`deny`",
            "`unica.cfe.patch_method`",
            "`unsupported_mechanism_variant`",
        ]
        for required in adr_headings + adr_anchors:
            with self.subTest(document="ADR 0008", required=required):
                self.assertIn(required, normalized_adr)

        self.assertNotIn('"mode": "advisory"', design)

        def table_after(document: str, marker: str, expected_header: list[str]) -> list[list[str]]:
            self.assertIn(marker, document)
            section = document.split(marker, 1)[1]
            lines: list[str] = []
            for line in section.splitlines():
                if line.startswith("|"):
                    lines.append(line)
                elif lines:
                    break
            self.assertGreaterEqual(len(lines), 3, f"missing table after {marker}")
            header = [cell.strip() for cell in lines[0].strip().strip("|").split("|")]
            self.assertEqual(header, expected_header)
            self.assertEqual(
                [cell.strip() for cell in lines[1].strip().strip("|").split("|")],
                ["---"] * len(expected_header),
            )
            rows = [
                [cell.strip() for cell in line.strip().strip("|").split("|")]
                for line in lines[2:]
            ]
            for row in rows:
                self.assertEqual(len(row), len(expected_header), f"invalid row after {marker}: {row}")
            return rows

        readiness_rows = table_after(
            design,
            "### Source Readiness Matrix",
            ["Analysis source", "Snapshot contract", "Public discovery outcome", "Receipt"],
        )
        readiness = {row[0].replace("`", ""): row[1:] for row in readiness_rows}
        self.assertEqual(len(readiness_rows), len(readiness), "duplicate source-readiness row")
        self.assertEqual(
            set(readiness),
            {
                "Platform XML configuration/extension",
                "EDT configuration with at least one recognized marker",
                "EDT configuration without a recognized marker",
                "EDT extension",
                "unknown format",
                "invalid mixed format",
                "external processor/report source kind",
            },
        )
        edt_ready = readiness["EDT configuration with at least one recognized marker"]
        self.assertIn("diagnostic marker snapshot", edt_ready[0])
        self.assertIn("`unsupported_source_format` source-readiness check", edt_ready[1])
        self.assertIn("never eligible", edt_ready[2])
        for source, reason in {
            "EDT configuration without a recognized marker": "unsupported_source_format",
            "EDT extension": "unsupported_source_format",
            "unknown format": "unknown_source_format",
            "invalid mixed format": "invalid_source_format",
            "external processor/report source kind": "unsupported_source_kind",
        }.items():
            self.assertEqual(readiness[source][0], "none")
            self.assertIn(reason, readiness[source][1])
            self.assertEqual(readiness[source][2], "never")

        role_rows = table_after(
            design,
            "Source readiness follows this first-match matrix",
            ["Role", "Source kind", "Format/layout", "Result"],
        )
        self.assertEqual(
            role_rows,
            [
                ["analysis", "external processor/report", "any", "`unsupported_source_kind`"],
                ["analysis", "configuration/extension", "Platform XML", "allowed authoritative capture"],
                ["analysis", "configuration", "EDT with at least one v1 marker", "allowed diagnostic capture"],
                ["analysis", "configuration", "EDT without a v1 marker", "`unsupported_source_format`"],
                ["analysis", "extension", "EDT", "`unsupported_source_format`"],
                ["analysis", "configuration/extension", "invalid", "`invalid_source_format`"],
                ["analysis", "configuration/extension", "unknown", "`unknown_source_format`"],
                ["destination", "any kind except extension", "any", "`unsupported_destination_kind`"],
                ["destination", "extension", "Platform XML", "allowed authoritative capture"],
                ["destination", "extension", "EDT, invalid, or unknown", "`unsupported_destination_format`"],
            ],
        )

        manifest_rows = table_after(
            design,
            "Manifest entries have two disjoint encodings:",
            ["Manifest tag", "Canonical fields"],
        )
        manifest = {row[0].strip("`"): row[1] for row in manifest_rows}
        self.assertEqual(len(manifest_rows), len(manifest), "duplicate manifest tag row")
        self.assertEqual(set(manifest), {"present", "absent_optional"})
        self.assertIn("byte length actually read", manifest["present"])
        self.assertIn("file-content SHA-256", manifest["present"])
        self.assertIn("no fictitious length or digest", manifest["absent_optional"])

        for required in [
            "ProjectSourceResolverPort::resolve_all",
            "one map-wide semantic digest shared by every returned identity",
            "covers only canonical effective topology",
            "missing or non-directory roots, dangling or live symlink/reparse roots",
            "same catalog and snapshot-bound verified reader",
            "`ConfigDumpInfo.xml` neither enters the manifest nor independently proves Platform XML format",
            "versioned registry of exact configuration/extension root `Ext` artifacts",
            "Unix walks every component through directory handles with `openat`/`O_NOFOLLOW`",
            "leaf with `O_NONBLOCK`",
            "Windows opens each single component relative to the already opened parent directory handle",
            "leaf-only path open plus final-prefix check is not sufficient",
            "final pass revalidates both present files and every `absent_optional` tombstone",
            "bounded by that file's previously captured byte length plus one",
            "normalizes the captured snapshot and every accepted evidence record",
            "can never create an identifier collision, change a verdict, or block the operation",
            "Source snapshot budgets never select prefixes",
            "DiscoveryError::SourceReadiness",
            "data.sourceReadiness",
            "DiscoveryError::SnapshotCapture",
            "data.snapshotCapture={reasonCode,retryable}",
        ]:
            with self.subTest(document="active discovery spec", snapshot_contract=required):
                self.assertIn(required, normalized_design)
        self.assertRegex(
            normalized_design,
            r"maxFiles=200000.*maxBytes=4GiB.*maxTraversalEntries=1600000.*"
            r"maxTraversalDepth=64.*maxXmlBytes=64MiB.*maxElapsed=120s",
        )
        edt_manifest_match = re.search(
            r"Its versioned v1\s+manifest contains(.*?); it does not recurse",
            design,
            re.DOTALL,
        )
        self.assertIsNotNone(edt_manifest_match, "missing bounded EDT v1 manifest clause")
        edt_manifest_clause = edt_manifest_match.group(1)
        edt_manifest_paths = re.findall(r"`([^`]+)`", edt_manifest_clause)
        self.assertEqual(
            edt_manifest_paths,
            [
                ".project",
                "DT-INF/PROJECT.PMF",
                "Configuration/Configuration.mdo",
                "src/Configuration/Configuration.mdo",
            ],
        )
        self.assertEqual(
            len(edt_manifest_paths),
            len(set(edt_manifest_paths)),
            "duplicate EDT v1 marker",
        )
        for marker in [
            "`provider=ProjectSourceResolverPort`",
            "`code=source_readiness`",
            "`state=skipped`",
            "`outcome=inconclusive`",
            "`coverage=unknown`",
            "`severity=blocking`",
            "`reasonCode=unsupported_source_format`",
            "`retryable=false`",
            "`proposal:<id>`",
        ]:
            self.assertIn(marker, design)
        self.assertIn(
            "The sole v1 orchestration check provider is `ProjectSourceResolverPort`",
            normalized_design,
        )
        retry_rows = table_after(
            design,
            "Snapshot failure classification is stable and exhaustive for v1:",
            ["`reasonCode`", "Failure evidence", "Retryable"],
        )
        retry_classes = {row[0].strip("`"): row[1:] for row in retry_rows}
        self.assertEqual(len(retry_rows), len(retry_classes), "duplicate snapshot reasonCode")
        self.assertEqual(
            [(row[0].strip("`"), row[2]) for row in retry_rows],
            [
                ("source_changed_during_capture", "yes"),
                ("unsafe_source_topology", "no"),
                ("source_snapshot_deadline", "yes"),
                ("source_io_unavailable", "yes"),
                ("malformed_source_material", "no"),
                ("unsupported_source_layout", "no"),
                ("invalid_source_path", "no"),
                ("source_snapshot_resource_limit", "no"),
                ("source_snapshot_invariant_violation", "no"),
            ],
        )
        self.assertIn("before/after", retry_classes["source_changed_during_capture"][0])
        self.assertIn("symlink/reparse", retry_classes["unsafe_source_topology"][0])
        self.assertIn("XML-byte", retry_classes["source_snapshot_resource_limit"][0])
        self.assertIn("source_changed_during_capture", design)
        self.assertIn("unsafe_source_topology", design)
        root_ext_clause = design.split("`SOURCE_ROOT_EXT_ARTIFACTS_V1`", 1)[1].split(
            "Unknown root-`Ext` files", 1
        )[0]
        root_ext_names = re.findall(r"`([^`]+)`", root_ext_clause)
        self.assertEqual(
            root_ext_names,
            [
                "ManagedApplicationModule.bsl",
                "OrdinaryApplicationModule.bsl",
                "SessionModule.bsl",
                "ExternalConnectionModule.bsl",
                "CommandInterface.xml",
                "ManagedApplicationCommandInterface.xml",
                "OrdinaryApplicationCommandInterface.xml",
                "ClientApplicationInterface.xml",
                "HomePageWorkArea.xml",
                "Help.xml",
            ],
        )
        self.assertEqual(
            len(root_ext_names),
            len(set(root_ext_names)),
            "duplicate root Ext v1 artifact",
        )
        self.assertIn("EDT uses only a bounded diagnostic snapshot", historical_plan)
        self.assertIn("typed `unknown` / `unsupported_source_format`", historical_plan)
        self.assertIn(
            "`data.sourceReadiness={reasonCode,retryable,sourceSet,role}`",
            historical_plan,
        )
        self.assertIn("`OperationData::SourceReadiness(SourceReadinessData)`", historical_plan)
        self.assertIn("`data.snapshotCapture={reasonCode,retryable}`", historical_plan)
        self.assertIn("`OperationData::SnapshotCapture(SnapshotCaptureData)`", historical_plan)

        expected_binding_matrix = {
            "Structural": (("contains", "defines"), "MetadataCatalogPort"),
            "EventSubscription": (("subscribes",), "MetadataCatalogPort"),
            "FormCommand": (("handles",), "FormInspectionPort"),
            "CommonCommand": (("handles",), "MetadataCatalogPort"),
            "ScheduledJob": (("handles",), "MetadataCatalogPort"),
            "HttpRoute": (("handles",), "MetadataCatalogPort"),
            "ExchangePlan": (("handles",), "MetadataCatalogPort"),
        }
        binding_violation = (
            "Every other `BindingDetails` x `FlowKind` x evidence-port combination "
            "is a `ProviderContractViolation` and must be rejected before evidence-graph promotion."
        )
        runtime_materiality = (
            "Runtime materiality follows evidence contribution: every runtime port present in "
            "`connection_ports` for the selected target is material, while other potential runtime "
            "ports are optional. If no runtime connection is established, a conclusive negative "
            "requires complete exact coverage from `MetadataCatalogPort`, `CallGraphPort`, and "
            "`FormInspectionPort`."
        )
        structural_semantics = (
            "`contains` and `defines` structural edges remain observed graph evidence; they never "
            "populate `connection_ports`, establish runtime reachability, or make a candidate "
            "actionable."
        )
        for document, text in {
            "active discovery spec": design,
            "historical Task 3 plan": historical_plan,
        }.items():
            normalized_text = " ".join(text.split())
            matrix_marker = "The canonical binding compatibility matrix is:"
            self.assertIn(matrix_marker, text)
            matrix_section = text.split(matrix_marker, 1)[1]
            matrix_lines: list[str] = []
            for line in matrix_section.splitlines():
                if line.startswith("|"):
                    matrix_lines.append(line)
                elif matrix_lines:
                    break
            self.assertGreaterEqual(len(matrix_lines), 2)
            self.assertEqual(
                matrix_lines[0],
                "| `BindingDetails` | Accepted `FlowKind` | Supplying evidence port |",
            )
            self.assertEqual(matrix_lines[1], "| --- | --- | --- |")
            actual_binding_matrix: dict[str, tuple[tuple[str, ...], str]] = {}
            for line in matrix_lines[2:]:
                cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
                self.assertEqual(len(cells), 3, f"invalid binding-matrix row: {line}")
                detail = cells[0].removeprefix("`").removesuffix("`")
                self.assertNotIn(
                    detail,
                    actual_binding_matrix,
                    f"duplicate binding detail in {document}: {detail}",
                )
                relations = tuple(re.findall(r"`([^`]+)`", cells[1]))
                provider = cells[2].removeprefix("`").removesuffix("`")
                actual_binding_matrix[detail] = (relations, provider)
            self.assertEqual(actual_binding_matrix, expected_binding_matrix)
            with self.subTest(document=document, binding_violation=True):
                self.assertIn(binding_violation, normalized_text)
            with self.subTest(document=document, runtime_materiality=True):
                self.assertIn(runtime_materiality, normalized_text)
            with self.subTest(document=document, structural_semantics=True):
                self.assertIn(structural_semantics, normalized_text)

        architecture_requirements = {
            "invariants": (
                normalized_invariants,
                [
                    "## Project Discovery",
                    "## Discovery Receipts And Guard",
                    "## Shadow Observations",
                    "DiscoverExtensionPointsUseCase",
                    "workspaceEpoch",
                    "dryRun: false",
                ],
            ),
            "change checklist": (
                normalized_checklist,
                [
                    "## Project Discovery And Discovery Receipts",
                    "## Shadow Observation And Replay",
                    "workspaceEpoch",
                    "unknown",
                ],
            ),
        }
        for document, (text, required_anchors) in architecture_requirements.items():
            for required in required_anchors:
                with self.subTest(document=document, required=required):
                    self.assertIn(required, text)

        arc42_requirements = {
            "05-building-block-view.md": [
                "## Discovery Evidence Blocks",
                "DiscoverExtensionPointsUseCase",
                "DiscoveryReceiptRepository",
                "ShadowObservationRepository",
            ],
            "06-runtime-view.md": [
                "## Applied Mutation",
                "## Project Discovery Explore",
                "## Project Discovery Validate",
                "## Shadow Observation And Replay",
                "receipt_busy",
                "stale_receipt_revision",
            ],
            "08-cross-cutting-concepts.md": [
                "## Typed Discovery Evidence",
                "## Discovery Receipts And Concurrency",
                "## Discovery Policy Rollout",
                "## Privacy-Preserving Shadow Evidence",
                "not_required",
                "advisory_only",
                "enforceable",
            ],
            "09-architecture-decisions.md": [
                "ADR-0008",
                "typed Project Discovery",
            ],
            "10-quality-requirements.md": [
                "## Determinism",
                "## Discovery Acceptance",
                "48",
                "unknown",
            ],
            "11-risks-and-technical-debt.md": [
                "## Active Risks",
                "## Mitigations",
                "observe",
                "unknown",
            ],
        }
        for file_name, required_phrases in arc42_requirements.items():
            for required in required_phrases:
                with self.subTest(document=file_name, required=required):
                    self.assertIn(required, arc42_files[file_name])

        privacy_prohibition = re.compile(
            r"\b(?:must\s+not|must\s+never|never)\s+(?:contain|store|include|persist)\b"
            r"(?=[^.]{0,240}\btask text\b)"
            r"(?=[^.]{0,240}\bsource text\b)[^.]*\.",
            flags=re.IGNORECASE,
        )
        privacy_documents = {
            "accepted design": normalized_design,
            "ADR 0008": normalized_adr,
            "invariants": normalized_invariants,
            "change checklist": normalized_checklist,
            "05-building-block-view.md": arc42_files["05-building-block-view.md"],
            "06-runtime-view.md": arc42_files["06-runtime-view.md"],
            "08-cross-cutting-concepts.md": arc42_files["08-cross-cutting-concepts.md"],
            "10-quality-requirements.md": arc42_files["10-quality-requirements.md"],
            "11-risks-and-technical-debt.md": arc42_files["11-risks-and-technical-debt.md"],
        }
        for document, text in privacy_documents.items():
            with self.subTest(document=document, contract="privacy prohibition"):
                self.assertRegex(text, privacy_prohibition)

        guard_order = normalized_design.split("### Guard Order", 1)[1].split("### Rollout Modes", 1)[0]
        runtime_order = arc42_files["06-runtime-view.md"].split("## Applied Mutation", 1)[1].split(
            "## Read Operation", 1
        )[0]
        ordered_stages = [
            "handler invocation",
            "typed mutation effects",
            "post-mutation source snapshot",
            "advance or revoke",
            "same exclusive receipt lease",
            "release the current receipt lease",
            "domain event emission",
            "cache invalidation",
            "other-receipt reconciliation",
            "workspace-service invalidation",
            "shadow observation",
            "result construction",
        ]
        for document, text in [("accepted design guard order", guard_order), ("arc42 runtime order", runtime_order)]:
            cursor = -1
            for stage in ordered_stages:
                with self.subTest(document=document, stage=stage):
                    cursor = text.find(stage, cursor + 1)
                    self.assertNotEqual(cursor, -1, f"missing or out-of-order stage: {stage}")

        def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
            result: dict[str, object] = {}
            for key, value in pairs:
                if key in result:
                    raise ValueError(f"duplicate JSON key: {key}")
                result[key] = value
            return result

        json_blocks, json_opener_count = parse_fenced_json_blocks(design)
        self.assertGreater(len(json_blocks), 0, "accepted design must contain JSON contract fixtures")
        self.assertEqual(len(json_blocks), json_opener_count)

        def assert_semantic_identifiers(value: object) -> None:
            if isinstance(value, list):
                for item in value:
                    assert_semantic_identifiers(item)
                return
            if not isinstance(value, dict):
                return

            if "analysisId" in value:
                self.assertRegex(value["analysisId"], r"^analysis_[0-9a-f]{64}$")
            for key in ["sourceFingerprint", "compositeSourceFingerprint"]:
                if key in value:
                    self.assertRegex(value[key], r"^sha256:[0-9a-f]{64}$")
            if "evidenceIds" in value:
                self.assertIsInstance(value["evidenceIds"], list)
                for evidence_id in value["evidenceIds"]:
                    self.assertRegex(evidence_id, r"^ev_[0-9a-f]{64}$")
            if "discoveryReceipt" in value:
                self.assertRegex(
                    value["discoveryReceipt"],
                    r"^discovery_receipt_[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                )
            receipt = value.get("receipt")
            if isinstance(receipt, dict):
                self.assertRegex(
                    receipt.get("id"),
                    r"^discovery_receipt_[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                )
            for child in value.values():
                assert_semantic_identifiers(child)

        parsed_blocks: list[object] = []
        for index, block in enumerate(json_blocks, start=1):
            with self.subTest(json_block=index):
                parsed = json.loads(block, object_pairs_hook=reject_duplicate_keys)
                assert_semantic_identifiers(parsed)
                parsed_blocks.append(parsed)

        check_fixtures = [
            value for value in parsed_blocks if isinstance(value, dict) and value.get("code") == "call_graph"
        ]
        self.assertEqual(len(check_fixtures), 1, "accepted design must contain one canonical Check fixture")
        self.assertEqual(
            set(check_fixtures[0]),
            {
                "code",
                "provider",
                "state",
                "outcome",
                "coverage",
                "severity",
                "affects",
                "reasonCode",
                "retryable",
                "details",
                "evidenceIds",
            },
        )

    def write_executable(self, tools_dir: Path, name: str, body: str) -> None:
        path = tools_dir / name
        path.write_text(body, encoding="utf-8")
        path.chmod(path.stat().st_mode | 0o755)

    def test_tool_help_contracts_pass_with_expected_cli_surface(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            self.write_executable(
                tools_dir,
                "bsl-analyzer",
                "#!/usr/bin/env sh\n"
                "printf '%s\\n' '--source-dir --format jsonl baseline --profile workspace reference "
                "--mode stdio --scenarios --json mcp serve analyze search smoke'\n",
            )
            self.write_executable(
                tools_dir,
                "rlm-bsl-index",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n",
            )
            self.write_executable(
                tools_dir,
                "rlm-tools-bsl",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_executable(
                tools_dir,
                "v8-runner",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner 0.5.1 version build'\n",
            )

            errors = module.check_tool_contracts(tools_dir)

        self.assertEqual(errors, [])

    def test_tool_help_contracts_accept_relative_tools_dir(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory(dir=Path.cwd()) as tmp:
            tools_dir = Path(tmp)
            self.write_executable(
                tools_dir,
                "bsl-analyzer",
                "#!/usr/bin/env sh\n"
                "printf '%s\\n' '--source-dir --format jsonl baseline --profile workspace reference "
                "--mode stdio --scenarios --json mcp serve analyze search smoke'\n",
            )
            self.write_executable(
                tools_dir,
                "rlm-bsl-index",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n",
            )
            self.write_executable(
                tools_dir,
                "rlm-tools-bsl",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_executable(
                tools_dir,
                "v8-runner",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner 0.5.1 version build'\n",
            )

            errors = module.check_tool_contracts(tools_dir.relative_to(Path.cwd()))

        self.assertEqual(errors, [])

    def test_tool_help_contracts_report_missing_expected_flag(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            self.write_executable(tools_dir, "bsl-analyzer", "#!/usr/bin/env sh\nprintf '%s\\n' 'analyze'\n")
            self.write_executable(tools_dir, "rlm-bsl-index", "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n")
            self.write_executable(
                tools_dir,
                "rlm-tools-bsl",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_executable(tools_dir, "v8-runner", "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner version build'\n")

            errors = module.check_tool_contracts(tools_dir)

        self.assertTrue(any("--source-dir" in error for error in errors), errors)

    def test_tool_help_contracts_report_missing_rlm_server_transport_surface(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            self.write_executable(
                tools_dir,
                "bsl-analyzer",
                "#!/usr/bin/env sh\n"
                "printf '%s\\n' '--source-dir --format jsonl baseline --profile workspace reference "
                "--mode stdio --scenarios --json mcp serve analyze search smoke'\n",
            )
            self.write_executable(tools_dir, "rlm-bsl-index", "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n")
            self.write_executable(tools_dir, "rlm-tools-bsl", "#!/usr/bin/env sh\nprintf '%s\\n' 'service'\n")
            self.write_executable(tools_dir, "v8-runner", "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner version build'\n")

            errors = module.check_tool_contracts(tools_dir)

        self.assertTrue(any("rlm-tools-bsl server" in error and "--transport" in error for error in errors), errors)

    def test_rlm_schema_contract_checks_tables_meta_and_columns_used_by_unica_sql(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "bsl_index.db"
            with sqlite3.connect(db_path) as conn:
                conn.execute("CREATE TABLE index_meta (key TEXT PRIMARY KEY, value TEXT)")
                conn.execute("INSERT INTO index_meta (key, value) VALUES ('builder_version', '14')")
                conn.execute(
                    "CREATE TABLE modules (id INTEGER, rel_path TEXT, object_name TEXT, "
                    "category TEXT, module_type TEXT)"
                )
                conn.execute(
                    "CREATE TABLE methods (id INTEGER, module_id INTEGER, name TEXT, type TEXT, "
                    "is_export INTEGER, line INTEGER, end_line INTEGER, params TEXT, loc INTEGER)"
                )
                conn.execute("CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name)")
                conn.execute(
                    "CREATE TABLE regions (id INTEGER, module_id INTEGER, name TEXT, "
                    "line INTEGER, end_line INTEGER)"
                )
                conn.execute("CREATE TABLE module_headers (module_id INTEGER, header_comment TEXT)")
                conn.execute(
                    "CREATE TABLE object_attributes (id INTEGER, object_name TEXT, category TEXT, "
                    "attr_name TEXT, attr_synonym TEXT, attr_type TEXT, attr_kind TEXT, "
                    "ts_name TEXT, source_file TEXT)"
                )
                conn.execute(
                    "CREATE TABLE role_rights (id INTEGER, role_name TEXT, object_name TEXT, "
                    "right_name TEXT, file TEXT)"
                )
                conn.execute(
                    "CREATE TABLE event_subscriptions (id INTEGER, name TEXT, synonym TEXT, "
                    "event TEXT, handler_module TEXT, handler_procedure TEXT, source_types TEXT, "
                    "source_count INTEGER, file TEXT)"
                )
                conn.execute(
                    "CREATE TABLE functional_options (id INTEGER, name TEXT, synonym TEXT, "
                    "location TEXT, content TEXT, file TEXT)"
                )
                conn.execute(
                    "CREATE TABLE predefined_items (id INTEGER, object_name TEXT, category TEXT, "
                    "item_name TEXT, item_synonym TEXT, item_code TEXT, types_json TEXT, "
                    "is_folder INTEGER, source_file TEXT)"
                )

            self.assertEqual(module.check_rlm_schema(db_path), [])

    def test_rlm_schema_contract_reports_missing_column(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "bsl_index.db"
            with sqlite3.connect(db_path) as conn:
                conn.execute("CREATE TABLE index_meta (key TEXT PRIMARY KEY, value TEXT)")
                conn.execute("INSERT INTO index_meta (key, value) VALUES ('builder_version', '14')")
                conn.execute("CREATE TABLE modules (id INTEGER, rel_path TEXT)")
                conn.execute("CREATE TABLE methods (id INTEGER, module_id INTEGER, name TEXT)")
                conn.execute("CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name)")
                conn.execute(
                    "CREATE TABLE regions (id INTEGER, module_id INTEGER, name TEXT, "
                    "line INTEGER, end_line INTEGER)"
                )
                conn.execute("CREATE TABLE module_headers (module_id INTEGER, header_comment TEXT)")

            errors = module.check_rlm_schema(db_path)

        self.assertTrue(any("modules.object_name" in error for error in errors), errors)

    def test_rlm_schema_contract_requires_metadata_tables_used_by_meta_profile(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "bsl_index.db"
            with sqlite3.connect(db_path) as conn:
                conn.execute("CREATE TABLE index_meta (key TEXT PRIMARY KEY, value TEXT)")
                conn.execute("INSERT INTO index_meta (key, value) VALUES ('builder_version', '14')")
                conn.execute(
                    "CREATE TABLE modules (id INTEGER, rel_path TEXT, object_name TEXT, "
                    "category TEXT, module_type TEXT)"
                )
                conn.execute(
                    "CREATE TABLE methods (id INTEGER, module_id INTEGER, name TEXT, type TEXT, "
                    "is_export INTEGER, line INTEGER, end_line INTEGER, params TEXT, loc INTEGER)"
                )
                conn.execute("CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name)")
                conn.execute(
                    "CREATE TABLE regions (id INTEGER, module_id INTEGER, name TEXT, "
                    "line INTEGER, end_line INTEGER)"
                )
                conn.execute("CREATE TABLE module_headers (module_id INTEGER, header_comment TEXT)")

            errors = module.check_rlm_schema(db_path)

        self.assertTrue(any("role_rights" in error for error in errors), errors)
        self.assertTrue(any("object_attributes" in error for error in errors), errors)
        self.assertTrue(any("functional_options" in error for error in errors), errors)

    def test_rlm_schema_contract_reports_old_builder_version(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "bsl_index.db"
            with sqlite3.connect(db_path) as conn:
                conn.execute("CREATE TABLE index_meta (key TEXT PRIMARY KEY, value TEXT)")
                conn.execute("INSERT INTO index_meta (key, value) VALUES ('builder_version', '12')")
                conn.execute(
                    "CREATE TABLE modules (id INTEGER, rel_path TEXT, object_name TEXT, "
                    "category TEXT, module_type TEXT)"
                )
                conn.execute(
                    "CREATE TABLE methods (id INTEGER, module_id INTEGER, name TEXT, type TEXT, "
                    "is_export INTEGER, line INTEGER, end_line INTEGER, params TEXT, loc INTEGER)"
                )
                conn.execute("CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name)")
                conn.execute(
                    "CREATE TABLE regions (id INTEGER, module_id INTEGER, name TEXT, "
                    "line INTEGER, end_line INTEGER)"
                )
                conn.execute("CREATE TABLE module_headers (module_id INTEGER, header_comment TEXT)")

            errors = module.check_rlm_schema(db_path)

        self.assertTrue(any("builder_version" in error and "14" in error for error in errors), errors)


if __name__ == "__main__":
    unittest.main()
