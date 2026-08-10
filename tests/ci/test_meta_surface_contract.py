from __future__ import annotations

import json
import re
import subprocess
import unittest
from collections import defaultdict
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
APPLICATION = REPO_ROOT / "crates/unica-coder/src/application/mod.rs"
METADATA = REPO_ROOT / "crates/unica-coder/src/application/metadata.rs"
TOOL_CONTRACTS = REPO_ROOT / "crates/unica-coder/src/application/tool_contracts.rs"
OPERATION_DESCRIPTORS = (
    REPO_ROOT / "crates/unica-coder/src/application/operation_descriptors.rs"
)
META_RUNTIME = REPO_ROOT / "crates/unica-coder/src/infrastructure/native_operations/meta"
META_DOMAIN = REPO_ROOT / "crates/unica-coder/src/domain/metadata"
FORMAT_GUARD = REPO_ROOT / "crates/unica-coder/src/infrastructure/format_guard.rs"
CF_RUNTIME = REPO_ROOT / "crates/unica-coder/src/infrastructure/native_operations/cf.rs"
TOOL_CONTEXT = REPO_ROOT / "crates/unica-coder/src/infrastructure/tool_context.rs"
META_PROPERTY_REGISTRY = (
    REPO_ROOT / "crates/unica-coder/src/domain/metadata/properties.rs"
)
META_OPERATION_REGISTRY = (
    REPO_ROOT / "crates/unica-coder/src/domain/metadata/operations.rs"
)
META_CAPABILITY_LEDGER = REPO_ROOT / "spec/architecture/meta-capability-parity.json"
META_MIGRATION = REPO_ROOT / "docs/migrations/0.12.0-meta-surface.md"
RETIRED_META_TYPE_REFERENCE = (
    REPO_ROOT
    / "tests/fixtures/provenance/retired_meta_dsl/meta-compile/reference"
)

DONOR_METADATA_KINDS = {
    "Catalog",
    "Document",
    "Enum",
    "Constant",
    "DefinedType",
    "Report",
    "DataProcessor",
    "BusinessProcess",
    "Task",
    "ExchangePlan",
    "CommonModule",
    "ScheduledJob",
    "EventSubscription",
    "DocumentJournal",
    "InformationRegister",
    "AccumulationRegister",
    "AccountingRegister",
    "CalculationRegister",
    "ChartOfAccounts",
    "ChartOfCharacteristicTypes",
    "ChartOfCalculationTypes",
    "HTTPService",
    "WebService",
}


def retired_meta_table_capabilities() -> dict[str, set[str]]:
    capabilities: dict[str, set[str]] = defaultdict(set)
    for path in sorted(RETIRED_META_TYPE_REFERENCE.glob("types-*.md")):
        current_kind: str | None = None
        in_table = False
        for line in path.read_text(encoding="utf-8").splitlines():
            heading = re.fullmatch(r"##\s+([A-Za-z][A-Za-z0-9]+)\s*", line)
            if heading:
                candidate = heading.group(1)
                current_kind = candidate if candidate in DONOR_METADATA_KINDS else None
                in_table = False
                continue
            if not line.startswith("|"):
                in_table = False
                continue
            cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
            if not in_table:
                in_table = True
                continue
            if not current_kind or not cells or re.fullmatch(r"[-: ]+", cells[0]):
                continue
            key = re.fullmatch(r"`([A-Za-z][A-Za-z0-9]*)`", cells[0])
            if key:
                capabilities[key.group(1)].add(current_kind)
    return dict(capabilities)


def registered_metadata_properties() -> set[str]:
    return set(
        re.findall(
            r'(?:public_name:\s*|(?:property|enum_property)\(\s*)'
            r'"([A-Za-z][A-Za-z0-9]*)"',
            META_PROPERTY_REGISTRY.read_text(encoding="utf-8"),
        )
    )


def rust_function(source: str, signature: str) -> str:
    start = source.index(signature)
    opening = source.index("{", start)
    depth = 0
    in_string = False
    escaped = False
    for index in range(opening, len(source)):
        char = source[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return source[opening + 1 : index]
    raise AssertionError(f"unclosed Rust function: {signature}")


def registered_tool_blocks() -> dict[str, str]:
    source = APPLICATION.read_text(encoding="utf-8")
    body = "\n".join(
        (
            rust_function(source, "pub fn tools()"),
            rust_function(source, "fn configuration_tools()"),
        )
    )
    blocks: dict[str, str] = {}
    for chunk in body.split("ToolSpec {")[1:]:
        match = re.search(r'name:\s*"([^"]+)"', chunk)
        if match:
            blocks[match.group(1)] = chunk.split("ToolSpec {", 1)[0]
    return blocks


class MetaSurfaceContractTests(unittest.TestCase):
    def test_subsystem_membership_has_one_registered_topology_owner(self) -> None:
        ports = (REPO_ROOT / "crates/unica-coder/src/application/ports.rs").read_text(
            encoding="utf-8"
        )
        info = (META_RUNTIME / "info.rs").read_text(encoding="utf-8")
        validation = (META_RUNTIME / "validation.rs").read_text(encoding="utf-8")
        validation_context = (META_RUNTIME / "validation_context.rs").read_text(
            encoding="utf-8"
        )
        results = (
            REPO_ROOT / "crates/unica-coder/src/domain/metadata/results.rs"
        ).read_text(encoding="utf-8")

        self.assertIn("enum MetadataSubsystemEvidence", ports)
        for marker in ("functional_subsystems", "interface_subsystems"):
            self.assertIn(marker, ports)
            self.assertIn(marker, results)
        self.assertIn("capture_registered_subsystem_topology", info)
        self.assertIn("MetadataObjectIdentity", info)
        self.assertIn("Uuid::parse_str", validation_context)
        self.assertIn("uuid: identity.object_uuid", info)
        self.assertNotIn("Uuid::parse_str", info)
        self.assertIn("functional_memberships_for", info)
        self.assertIn("interface_memberships_for", info)
        self.assertNotIn("functional_memberships(target.as_str())", info)
        self.assertNotIn("interface_memberships(target.as_str())", info)
        for retired in (
            "typed_subsystem_images",
            "subsystem_names_from_logical_path",
            "meta_validate_subsystem_command_interface_scan",
            "SubsystemDescriptorFacts",
        ):
            self.assertNotIn(retired, info + validation + validation_context)

    def test_meta_info_public_contract_distinguishes_empty_from_unavailable_memberships(
        self,
    ) -> None:
        skill = (
            REPO_ROOT / "plugins/unica/skills/meta-info/SKILL.md"
        ).read_text(encoding="utf-8")
        ledger = (
            REPO_ROOT / "spec/architecture/tool-surface.md"
        ).read_text(encoding="utf-8")
        review = json.loads(
            (
                REPO_ROOT / "spec/architecture/tool-surface-review.json"
            ).read_text(encoding="utf-8")
        )["unica.meta.info"]
        ledger_section = ledger.split("### `unica.meta.info`", 1)[1].split(
            "### `unica.meta.remove`", 1
        )[0]

        for text in (skill, ledger_section, json.dumps(review, ensure_ascii=False)):
            for marker in (
                "functionalSubsystems",
                "interfaceSubsystems",
                "UUID",
                "`[]`",
                "provider_unavailable",
            ):
                self.assertIn(marker, text)

    def test_retired_dsl_capabilities_are_accounted_for(self) -> None:
        donor = retired_meta_table_capabilities()
        self.assertTrue(donor, "retired Meta DSL type tables yielded no capabilities")
        self.assertTrue(
            META_CAPABILITY_LEDGER.exists(),
            "spec/architecture/meta-capability-parity.json is missing",
        )
        ledger = json.loads(META_CAPABILITY_LEDGER.read_text(encoding="utf-8"))
        self.assertIsInstance(ledger, list, "Meta capability ledger must be a JSON array")

        entries: dict[str, dict[str, object]] = {}
        for index, entry in enumerate(ledger):
            self.assertIsInstance(entry, dict, f"ledger entry {index} must be an object")
            legacy_key = entry.get("legacyKey")
            self.assertIsInstance(
                legacy_key, str, f"ledger entry {index} has no string legacyKey"
            )
            self.assertNotIn(legacy_key, entries, f"duplicate legacyKey: {legacy_key}")
            entries[legacy_key] = entry

        self.assertEqual(
            set(entries),
            set(donor),
            "ledger keys differ from the retired types-*.md capability tables",
        )

        registry = registered_metadata_properties()
        operation_registry = META_OPERATION_REGISTRY.read_text(encoding="utf-8")
        for legacy_key, legacy_kinds in sorted(donor.items()):
            entry = entries[legacy_key]
            with self.subTest(legacy_key=legacy_key):
                status = entry.get("status")
                self.assertIn(status, {"supported", "removed"})
                contract_field = (
                    "typedOperation" if status == "supported" else "removalReason"
                )
                other_field = (
                    "removalReason" if status == "supported" else "typedOperation"
                )
                self.assertEqual(
                    set(entry),
                    {"legacyKey", "legacyKinds", "status", contract_field},
                )
                self.assertNotIn(other_field, entry)
                self.assertEqual(entry.get("legacyKinds"), sorted(legacy_kinds))
                contract = entry.get(contract_field)
                self.assertIsInstance(contract, str)
                self.assertTrue(contract.strip())
                if status == "supported":
                    prefix = "unica.meta.edit.setProperties."
                    if contract.startswith(prefix):
                        self.assertIn(contract.removeprefix(prefix), registry)
                    else:
                        operation_evidence = {
                            "unica.meta.edit.add.enumValues": 'Self::EnumValues => "enumValues"',
                            "unica.meta.edit.editRelations.owners": 'Self::Owners => "owners"',
                            "unica.meta.edit.editRelations.registerRecords": (
                                'Self::RegisterRecords => "registerRecords"'
                            ),
                        }
                        self.assertIn(contract, operation_evidence)
                        self.assertIn(operation_evidence[contract], operation_registry)
                else:
                    self.assertNotRegex(contract, r"(?i)^\s*(?:the )?dsl (?:was )?removed")

    def test_removed_register_type_is_absent_from_public_registry(self) -> None:
        ledger = json.loads(META_CAPABILITY_LEDGER.read_text(encoding="utf-8"))
        register_type = next(
            entry for entry in ledger if entry["legacyKey"] == "registerType"
        )

        self.assertEqual(register_type["status"], "removed")
        self.assertNotIn("RegisterType", registered_metadata_properties())

    def test_live_rust_guidance_never_advertises_retired_meta_routes(self) -> None:
        retired_route = re.compile(
            r"(?<![A-Za-z0-9_])meta-(?:compile|validate|profile)\b|"
            r"unica\.meta\.(?:compile|validate|profile)\b"
        )
        production_surfaces = (
            (CF_RUNTIME, "pub(crate) fn edit_cf_with_data"),
            (TOOL_CONTEXT, "fn validates_compile_preview_like_apply"),
        )
        violations: list[str] = []
        for path, signature in production_surfaces:
            production = rust_function(path.read_text(encoding="utf-8"), signature)
            if retired_route.search(production):
                violations.append(
                    f"{path.relative_to(REPO_ROOT).as_posix()}: {signature}"
                )

        self.assertEqual(
            violations,
            [],
            "live Rust error/help paths still advertise retired Meta routes:\n"
            + "\n".join(violations),
        )

    def test_live_meta_runtime_has_no_file_or_string_dsl_grammar(self) -> None:
        sources = sorted(
            [
                path
                for path in META_RUNTIME.glob("*.rs")
                if not path.name.endswith("_tests.rs")
            ]
            + list(META_DOMAIN.glob("*.rs"))
        )
        retired_grammar = {
            "module-wide dead-or-unused suppression": re.compile(
                r"^#!\[allow\([^\]]*(?:dead_code|unused_imports|unused\b)",
                re.MULTILINE,
            ),
            "file selector": re.compile(r"\b(?:DefinitionFile|JsonPath)\b"),
            "batch separator": re.compile(re.escape('.split(";;")')),
            "attribute pipe flags": re.compile(re.escape("splitn(2, '|')")),
            "string type grammar": re.compile(re.escape('strip_prefix("String(")')),
            "number type grammar": re.compile(re.escape('strip_prefix("Number(")')),
            "string operation value": re.compile(r"requires Value|Name: Type"),
            "definition handlers": re.compile(r"\bmeta_edit_definition_[a-z0-9_]+"),
            "legacy compile helper": re.compile(r"\bmeta_compile_[a-z0-9_]+"),
            "legacy JSON template projection": re.compile(
                r"fn get\(&self, key: &str\) -> Option<&Value>"
            ),
            "legacy JSON customization helpers": re.compile(
                r"\b(?:bool_arg_from_json|meta_compile_string_list|"
                r"normalize_meta_enum_value)\b"
            ),
            "orphan Meta compatibility wrapper": re.compile(
                r"\b(?:MetaEditAfterLineNumberLengthPolicyHook|"
                r"with_meta_edit_after_line_number_length_policy_hook|"
                r"run_meta_edit_after_line_number_length_policy_hook|"
                r"meta_edit_projected_diff|meta_validate_one|"
                r"inspect_meta_validation_reads|PublicOwnerAware|"
                r"MetaValidationOwnerContext|MetaValidationReadInspection|"
                r"metadata_validation_subject_from_paths|metadata_validation_run|"
                r"(?:analyze_meta_info|remove_metadata_object)[A-Za-z0-9_]*|"
                r"metadata_object_registered)\b"
            ),
        }

        violations: list[str] = []
        for path in sources:
            source = path.read_text(encoding="utf-8")
            for contract, pattern in retired_grammar.items():
                if pattern.search(source):
                    violations.append(f"{path.relative_to(REPO_ROOT)}: {contract}")
        format_guard = FORMAT_GUARD.read_text(encoding="utf-8")
        for operation in ("meta-edit", "meta-info", "meta-remove", "meta-validate"):
            if f'"{operation}"' in format_guard:
                violations.append(
                    f"{FORMAT_GUARD.relative_to(REPO_ROOT)}: retired {operation} route"
                )

        self.assertEqual(
            violations,
            [],
            "live Meta runtime still contains the retired file/string grammar:\n"
            + "\n".join(violations),
        )

    def test_tracked_tree_has_no_executable_legacy_meta_dsl_bridge(self) -> None:
        tracked = subprocess.run(
            ["git", "ls-files", "-z"],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
        ).stdout.decode("utf-8").split("\0")
        excluded_prefixes = (
            "docs/design/",
            "docs/plans/",
            "spec/decisions/",
            "tests/fixtures/unica_mcp_script_parity/cc-1c-skills/",
            # Reviewed donor adaptations are retained only in a non-executable
            # provenance archive; they do not define the current Meta contract.
            "tests/fixtures/provenance/retired_meta_dsl/",
        )
        retired_files = {
            "crates/unica-coder/src/infrastructure/native_operations/meta/compile_tests.rs",
            "crates/unica-coder/src/infrastructure/native_operations/meta/legacy_dsl.rs",
            "plugins/unica/references/specs/meta-dsl-spec.md",
            "plugins/unica/skills/meta-edit/child-operations.md",
            "plugins/unica/skills/meta-edit/json-dsl.md",
            "plugins/unica/skills/meta-edit/properties-reference.md",
        }
        retired_identifiers = re.compile(
            r"call_legacy_metadata_tool_for_tests|"
            r"legacy_metadata_tool_spec_for_tests|"
            r"compile_legacy_metadata_fixture|"
            r"LEGACY_META_TEST_DESCRIPTORS|"
            r"parse_meta_edit_dsl_input|"
            r"meta_compile_object_xml|"
            r"meta_edit_apply_inline_operation|"
            r"meta_edit_apply_definition"
        )

        violations: list[str] = []
        text_suffixes = {
            ".bsl",
            ".js",
            ".json",
            ".md",
            ".mjs",
            ".ps1",
            ".py",
            ".rs",
            ".sh",
            ".toml",
            ".ts",
            ".txt",
            ".xml",
            ".yaml",
            ".yml",
        }
        for relative in tracked:
            if not relative or relative.startswith(excluded_prefixes):
                continue
            if relative == "tests/ci/test_meta_surface_contract.py":
                continue
            path = REPO_ROOT / relative
            if relative in retired_files and path.exists():
                violations.append(relative)
                continue
            if path.suffix.lower() not in text_suffixes:
                continue
            if not path.exists():
                continue
            try:
                source = path.read_text(encoding="utf-8")
            except (UnicodeDecodeError, OSError):
                continue
            if retired_identifiers.search(source):
                violations.append(relative)

        self.assertEqual(
            sorted(set(violations)),
            [],
            "tracked executable Meta DSL bridges remain:\n" + "\n".join(violations),
        )

    def test_registry_is_exactly_the_four_typed_metadata_handlers(self) -> None:
        blocks = registered_tool_blocks()
        meta = {name: block for name, block in blocks.items() if name.startswith("unica.meta.")}
        expected = {
            "unica.meta.info": ("false", "Info"),
            "unica.meta.add": ("true", "Add"),
            "unica.meta.edit": ("true", "Edit"),
            "unica.meta.remove": ("true", "Remove"),
        }

        self.assertEqual(set(meta), set(expected))
        for name, (mutating, operation) in expected.items():
            with self.subTest(name=name):
                block = meta[name]
                self.assertRegex(block, rf"mutating:\s*{mutating},")
                self.assertIn("handler: ToolHandler::Metadata {", block)
                self.assertRegex(
                    block,
                    rf"operation:\s*metadata::MetadataOperation::{operation},",
                )
                self.assertNotIn("ToolHandler::NativeOperation", block)
                self.assertNotIn("ToolHandler::CodeIntelligence", block)

        source = APPLICATION.read_text(encoding="utf-8")
        tools_body = rust_function(source, "pub fn tools()") + rust_function(
            source, "fn configuration_tools()"
        )
        self.assertNotIn("CodeIntelligenceOperation::ObjectProfile", tools_body)

        add_description = meta["unica.meta.add"]
        self.assertNotIn("Create one minimal metadata object", add_description)
        self.assertIn("typed internal template", add_description)
        self.assertIn("configure it atomically", add_description)
        self.assertIn("ordered operations", add_description)

    def test_meta_migration_publishes_protocol_and_capability_boundaries(self) -> None:
        migration = META_MIGRATION.read_text(encoding="utf-8")
        compact_migration = " ".join(migration.split())

        self.assertIn("structuredContent", migration)
        self.assertIn("isError == !structuredContent.ok", compact_migration)
        self.assertIn("data.effects", migration)
        self.assertIn("полный XML", migration)
        capabilities = json.loads(META_CAPABILITY_LEDGER.read_text(encoding="utf-8"))
        supported = sum(entry["status"] == "supported" for entry in capabilities)
        removed = sum(entry["status"] == "removed" for entry in capabilities)
        self.assertEqual(len(capabilities), 97)
        self.assertIn(f"{len(capabilities)} ключей", compact_migration)
        self.assertIn(f"{supported} поддерживаемых", compact_migration)
        self.assertIn(f"{removed} намеренно снятая", compact_migration)
        self.assertIn(
            "../../spec/architecture/meta-capability-parity.json", migration
        )

    def test_schema_path_has_exact_lower_camel_arguments_and_typed_edit_items(self) -> None:
        metadata = METADATA.read_text(encoding="utf-8")
        schema = rust_function(metadata, "pub(crate) fn metadata_input_schema")
        expected_required = {
            "Info": '["sourceSet", "metadataPath"]',
            "Add": '["sourceSet", "kind", "name"]',
            "Edit": '["sourceSet", "metadataPath", "operations"]',
            "Remove": '["sourceSet", "metadataPath"]',
        }
        expected_properties = {
            "Info": {"sourceSet", "metadataPath", "sections", "limit"},
            "Add": {"sourceSet", "kind", "name", "operations", "dryRun"},
            "Edit": {"sourceSet", "metadataPath", "operations", "dryRun"},
            "Remove": {"sourceSet", "metadataPath", "dryRun", "force", "confirm"},
        }

        for operation, required in expected_required.items():
            start = schema.index(f"MetadataOperation::{operation} =>")
            following = [
                schema.find(f"MetadataOperation::{candidate} =>", start + 1)
                for candidate in expected_required
            ]
            following = [position for position in following if position >= 0]
            end = min(following, default=len(schema))
            arm = schema[start:end]
            properties = {"sourceSet"}
            properties.update(
                re.findall(r'properties\.insert\(\s*"([^"]+)"', arm)
            )
            with self.subTest(operation=operation):
                self.assertEqual(properties, expected_properties[operation])
                self.assertIn(f"vec!{required}", arm)

        info_start = schema.index("MetadataOperation::Info =>")
        info_end = schema.index("MetadataOperation::Add =>", info_start)
        info_arm = schema[info_start:info_end]
        self.assertRegex(
            info_arm,
            re.compile(
                r'"sections"\.into\(\),\s*json!\(\{.*?"default":\s*\[\],',
                re.DOTALL,
            ),
        )
        self.assertRegex(
            info_arm,
            re.compile(
                r'"limit"\.into\(\),\s*json!\(\{\s*'
                r'"type": "integer",\s*"minimum": 1,\s*'
                r'"maximum": 50,\s*"default": 20,',
                re.DOTALL,
            ),
        )

        # ADR-0025: the operation union is published directly as
        # `properties.operations.items`, so a host that renders only
        # `properties` still sees the discriminated variants. It carries no
        # conditional composition and no owner-kind branching.
        edit_items = rust_function(metadata, "fn host_visible_operation_schema()")
        self.assertIn('"oneOf"', edit_items)
        for composition in ("anyOf", "allOf", '"if"', '"then"'):
            self.assertNotIn(composition, edit_items)
        schema_fn = rust_function(metadata, "pub(crate) fn metadata_input_schema")
        self.assertIn('"items": host_visible_operation_schema()', schema_fn)
        self.assertNotIn('"allOf"', schema_fn)
        for forbidden in (
            "JsonPath",
            "DefinitionFile",
            "Operation",
            "Value",
            "ObjectPath",
            "ConfigDir",
            "sourceDir",
            "path",
        ):
            self.assertNotIn(f'"{forbidden}"', schema + edit_items)

        self.assertEqual(
            len(
                re.findall(
                    r'"dryRun"\.into\(\).*?"default":\s*true',
                    schema,
                    re.DOTALL,
                )
            ),
            3,
        )
        success = rust_function(metadata, "fn metadata_success")
        self.assertIn("adapter: AdapterOutcome::ok(summary)", success)
        self.assertIn("data: Some(data)", success)

    def test_metadata_handlers_bypass_native_alias_and_descriptor_contracts(self) -> None:
        contracts = TOOL_CONTRACTS.read_text(encoding="utf-8")
        schema_path = rust_function(contracts, "pub fn input_schema_for_tool")
        validation_path = rust_function(contracts, "pub fn validate_tool_arguments")
        self.assertIn("ToolHandler::Metadata", schema_path)
        self.assertIn("metadata_input_schema(operation)", schema_path)
        self.assertIn("ToolHandler::Metadata", validation_path)
        self.assertIn("parse_metadata_request(operation, args)", validation_path)

        descriptors = OPERATION_DESCRIPTORS.read_text(encoding="utf-8")
        registry = descriptors.index("NATIVE_OPERATION_DESCRIPTORS")
        public_descriptors = descriptors[registry:]
        for retired in (
            "meta-compile",
            "meta-edit",
            "meta-info",
            "meta-remove",
            "meta-validate",
        ):
            self.assertNotIn(f'"{retired}"', public_descriptors)


if __name__ == "__main__":
    unittest.main()
