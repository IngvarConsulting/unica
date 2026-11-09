from __future__ import annotations

import json
import re
import unittest
from collections import defaultdict
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
APPLICATION = REPO_ROOT / "crates/unica-application/src/application/mod.rs"
METADATA = REPO_ROOT / "crates/unica-application/src/application/metadata.rs"
TOOL_CONTRACTS = REPO_ROOT / "crates/unica-application/src/application/tool_contracts.rs"
OPERATION_DESCRIPTORS = (
    REPO_ROOT / "crates/unica-application/src/application/operation_descriptors.rs"
)
META_RUNTIME = REPO_ROOT / "crates/unica-coder/src/infrastructure/native_operations/meta"
META_DOMAIN = REPO_ROOT / "crates/unica-metadata/src"
META_REQUEST = META_DOMAIN / "request.rs"
META_DESIGNER = REPO_ROOT / "crates/unica-metadata-designer/src"
# One file per property; `build.rs` of unica-metadata assembles the registry.
META_PROPERTY_REGISTRY = REPO_ROOT / "crates/unica-metadata/src/properties"
META_OPERATION_REGISTRY = (
    REPO_ROOT / "crates/unica-metadata/src/operations.rs"
)
META_CAPABILITY_LEDGER = REPO_ROOT / "tests/fixtures/migration/meta-capability-parity.json"
META_INFO_FIXTURE_MANIFEST = (
    REPO_ROOT / "tests/fixtures/platform_8_3_27/meta_info/manifest.json"
)
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
            "\n".join(
                path.read_text(encoding="utf-8")
                for path in sorted(META_PROPERTY_REGISTRY.glob("*.rs"))
            ),
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
    def test_meta_observation_and_mutation_capability_contract_is_published(
        self,
    ) -> None:
        surface = (
            REPO_ROOT / "docs/tool-surface.md"
        ).read_text(encoding="utf-8")
        surface_review = json.loads(
            (
                REPO_ROOT / "tests/fixtures/v013/tool-surface-review.json"
            ).read_text(encoding="utf-8")
        )
        meta_info = (
            REPO_ROOT / "plugins/unica/skills/meta-info/SKILL.md"
        ).read_text(encoding="utf-8")
        meta_edit = (
            REPO_ROOT / "plugins/unica/skills/meta-edit/SKILL.md"
        ).read_text(encoding="utf-8")
        format_spec = (
            REPO_ROOT / "plugins/unica/references/specs/1c-config-objects-spec.md"
        ).read_text(encoding="utf-8")

        for text in (surface, meta_info):
            for marker in ("mutationCapability", "editable", "readOnly", "uuid"):
                self.assertIn(marker, text)
        self.assertIn('{"kind": "uuid"}', meta_edit)
        self.assertIn("Forms/<Имя>.xml", format_spec)
        self.assertIn("Templates/<Имя>.xml", format_spec)
        self.assertNotIn("Forms/<Имя>/<Имя>.xml", format_spec)
        self.assertNotIn("Templates/<Имя>/<Имя>.xml", format_spec)

        command_section = format_spec.split("### 6.5. Команда (Command)", 1)[1].split(
            "---", 1
        )[0]
        self.assertIn(
            "Отдельного файла `Commands/<Имя>.xml` платформа не создаёт",
            command_section,
        )
        self.assertIn("Commands/<Имя>/Ext/CommandModule.bsl", command_section)

        html_section = format_spec.split("Для `HTMLDocument`", 1)[1].split(
            "### 6.5.", 1
        )[0]
        self.assertIn("HTML-страница не разбирается как XML", html_section)
        self.assertIn("кодировку UTF-8", html_section)
        self.assertIn("сохраняется побайтово", html_section)

        meta_info_compact = " ".join(meta_info.split())
        self.assertIn(
            "Синтаксически корректный, но ещё не моделируемый платформенный QName",
            meta_info_compact,
        )
        self.assertIn(
            "даёт warning, не превращая весь вызов в ошибку", meta_info_compact
        )

        review = surface_review["unica.meta.info"]["result"]
        self.assertEqual(review["target"], "достигнут")
        for marker in (
            "встроенные команды",
            "mutationCapability: editable | readOnly",
            "UUID представлен вариантом `uuid`",
            "QName оставляет только свой элемент `incomplete` с warning",
            "HTML-страницы зарегистрированных макетов удерживаются как UTF-8 без XML-разбора",
        ):
            self.assertIn(marker, review["now"])

    def test_subsystem_membership_has_one_registered_topology_owner(self) -> None:
        ports = (REPO_ROOT / "crates/unica-application/src/application/ports.rs").read_text(
            encoding="utf-8"
        )
        info_binding = (META_RUNTIME / "info.rs").read_text(encoding="utf-8")
        info = (META_DESIGNER / "info_read.rs").read_text(encoding="utf-8")
        validation = (META_DESIGNER / "validation.rs").read_text(encoding="utf-8")
        validation_context = (META_DESIGNER / "validation_context.rs").read_text(
            encoding="utf-8"
        )
        subject = (META_DOMAIN / "validation_subject.rs").read_text(encoding="utf-8")
        results = (META_DOMAIN / "results.rs").read_text(encoding="utf-8")
        topology = (META_DESIGNER / "subsystem_topology.rs").read_text(encoding="utf-8")

        self.assertIn("enum MetadataSubsystemEvidence", subject)
        self.assertNotIn("enum MetadataSubsystemEvidence", ports)
        self.assertIn("unica_metadata::validation_subject::MetadataValidationSubject", ports)
        for marker in ("functional_subsystems", "interface_subsystems"):
            self.assertIn(marker, subject)
            self.assertIn(marker, results)
        projection = rust_function(info, "pub fn read_typed_meta_info")
        evidence = rust_function(info, "fn typed_subsystem_evidence")
        retained_open = rust_function(info_binding, "fn open_retained_read")
        topology_read = rust_function(topology, "pub fn read_registered_subsystem_topology")
        native_read = rust_function(info_binding, "pub(crate) fn read_typed_meta_info")

        self.assertIn("MetadataObjectIdentity", projection)
        identity_parser = rust_function(
            validation_context, "fn inspect_metadata_image_identity_for_kinds"
        )
        self.assertIn("Uuid::parse_str", identity_parser)
        self.assertIn("uuid: identity.object_uuid", projection)
        self.assertNotIn("Uuid::parse_str", info)
        self.assertIn("functional_memberships_for(identity)", evidence)
        self.assertIn("interface_memberships_for(identity)", evidence)
        self.assertNotIn("functional_memberships(target.as_str())", info)
        self.assertNotIn("interface_memberships(target.as_str())", info)
        self.assertIn(".open_retained_read(", evidence)
        self.assertIn("crate::subsystem_topology::read_registered_subsystem_topology", evidence)
        self.assertLess(
            evidence.index(".open_retained_read("),
            evidence.index("crate::subsystem_topology::read_registered_subsystem_topology"),
        )
        # Only the depth bounds the registered walk; its size does not.
        self.assertIn("maximum_depth: SUBSYSTEM_ADDRESS_MAX_DEPTH * 2", evidence)
        for retired in ("maximum_entries", "maximum_files", "maximum_bytes"):
            self.assertNotIn(retired, evidence)
        self.assertIn("reader.complete(&mut checkpoint)?", topology_read)
        self.assertIn("reader.as_mut()", topology_read)
        self.assertIn(".read_regular_file(&configuration_path, &mut checkpoint)", topology_read)
        self.assertIn(
            "unica_workspace::read::RetainedRootSecureRead::open(root, limits, checkpoint)",
            retained_open,
        )
        self.assertIn("unica_metadata_designer::info_read::read_typed_meta_info(", native_read)
        self.assertIn("&MetadataDependencyReadAdapter::new(context)", native_read)
        self.assertNotIn("ProjectDiscoveryReader", evidence + topology_read)
        for retired in (
            "typed_subsystem_images",
            "subsystem_names_from_logical_path",
            "meta_validate_subsystem_command_interface_scan",
            "SubsystemDescriptorFacts",
        ):
            self.assertNotIn(retired, info + validation + validation_context + info_binding)

    def test_meta_info_public_contract_distinguishes_empty_from_unavailable_memberships(
        self,
    ) -> None:
        skill = (
            REPO_ROOT / "plugins/unica/skills/meta-info/SKILL.md"
        ).read_text(encoding="utf-8")
        ledger = (
            REPO_ROOT / "docs/tool-surface.md"
        ).read_text(encoding="utf-8")
        review = json.loads(
            (
                REPO_ROOT / "tests/fixtures/v013/tool-surface-review.json"
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

    def test_meta_info_complete_read_contract_is_synchronized(self) -> None:
        skill = (REPO_ROOT / "plugins/unica/skills/meta-info/SKILL.md").read_text(
            encoding="utf-8"
        )
        ledger = (REPO_ROOT / "docs/tool-surface.md").read_text(
            encoding="utf-8"
        )
        review = json.loads(
            (REPO_ROOT / "tests/fixtures/v013/tool-surface-review.json").read_text(
                encoding="utf-8"
            )
        )["unica.meta.info"]
        manifest = json.loads(META_INFO_FIXTURE_MANIFEST.read_text(encoding="utf-8"))
        platform_corpus = (
            REPO_ROOT / "crates/unica-product/tests/format_8_3_27_xml_corpus.rs"
        ).read_text(encoding="utf-8")

        self.assertEqual(len(manifest["kinds"]), len(DONOR_METADATA_KINDS))
        self.assertEqual(
            {entry["kind"] for entry in manifest["kinds"]},
            DONOR_METADATA_KINDS,
        )
        self.assertEqual(
            sorted(
                fixture
                for entry in manifest["kinds"]
                for fixture in entry.get("edgeFixtures", [])
            ),
            sorted(
                [
                    "edge/constant-type.xml",
                    "edge/defined-type.xml",
                    "edge/scheduled-job.xml",
                    "edge/calculation-register.xml",
                    "edge/chart-of-accounts-child-kinds.xml",
                    "edge/http-service.xml",
                    "edge/http-service-configurator.xml",
                    "edge/web-service.xml",
                    "edge/web-service-configurator.xml",
                    "edge/task-addressing.xml",
                    "edge/catalog-child-kinds.xml",
                    "edge/document-journal-child-kinds.xml",
                    "edge/enum-child-kinds.xml",
                ]
            ),
        )
        for entry in manifest["kinds"]:
            self.assertEqual(entry["mainFixture"], "canonical-template")
            self.assertIsInstance(entry["platformCase"], str)
            self.assertIn(f'"{entry["platformCase"]}"', platform_corpus)
            for fixture in entry.get("edgeFixtures", []):
                self.assertTrue((META_INFO_FIXTURE_MANIFEST.parent / fixture).is_file())

        surfaces = (skill, ledger, json.dumps(review, ensure_ascii=False))
        for text in surfaces:
            for marker in (
                "details",
                "UUID",
                "provider_unavailable",
            ):
                self.assertIn(marker, text)

    def test_retired_dsl_capabilities_are_accounted_for(self) -> None:
        donor = retired_meta_table_capabilities()
        self.assertTrue(donor, "retired Meta DSL type tables yielded no capabilities")
        self.assertTrue(
            META_CAPABILITY_LEDGER.exists(),
            "tests/fixtures/migration/meta-capability-parity.json is missing",
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
                            "unica.meta.edit.editRelations.source": (
                                'Self::Source => "source"'
                            ),
                        }
                        self.assertIn(contract, operation_evidence)
                        self.assertIn(operation_evidence[contract], operation_registry)
                else:
                    self.assertNotRegex(contract, r"(?i)^\s*(?:the )?dsl (?:was )?removed")

    def test_register_type_is_a_typed_registry_property(self) -> None:
        ledger = json.loads(META_CAPABILITY_LEDGER.read_text(encoding="utf-8"))
        register_type = next(
            entry for entry in ledger if entry["legacyKey"] == "registerType"
        )

        self.assertEqual(register_type["status"], "supported")
        self.assertEqual(
            register_type["typedOperation"],
            "unica.meta.edit.setProperties.RegisterType",
        )
        self.assertIn("RegisterType", registered_metadata_properties())

    def test_registry_is_exactly_the_three_typed_metadata_handlers(self) -> None:
        # `unica.meta.remove` is retired:
        # removal is `unica.apply object.remove` on the canonical surface.
        blocks = registered_tool_blocks()
        meta = {name: block for name, block in blocks.items() if name.startswith("unica.meta.")}
        expected = {
            "unica.meta.info": ("Read", "Info"),
            "unica.meta.add": ("Mutation", "Add"),
            "unica.meta.edit": ("Mutation", "Edit"),
        }

        self.assertEqual(set(meta), set(expected))
        for name, (execution, operation) in expected.items():
            with self.subTest(name=name):
                block = meta[name]
                self.assertRegex(
                    block,
                    rf"execution:\s*ToolExecution::{execution},",
                )
                self.assertRegex(
                    block,
                    r"result_contract:\s*ResultContract::Typed,",
                )
                self.assertIn("handler: ToolHandler::Metadata {", block)
                self.assertRegex(
                    block,
                    rf"operation:\s*unica_metadata::request::MetadataOperation::{operation},",
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

    def test_schema_path_has_exact_lower_camel_arguments_and_typed_edit_items(self) -> None:
        metadata = META_REQUEST.read_text(encoding="utf-8")
        schema = rust_function(metadata, "pub fn metadata_input_schema")
        expected_required = {
            "Info": '["sourceSet", "metadataPath"]',
            "Add": '["sourceSet", "kind", "name"]',
            "Edit": '["sourceSet", "metadataPath", "operations"]',
        }
        # `cwd` адресует рабочее пространство и обязателен на всей поверхности:
        # упакованный сервер стартует в каталоге плагина, а
        # наследовать рабочий каталог вызова запрещено.
        expected_properties = {
            "Info": {"cwd", "sourceSet", "metadataPath", "sections", "limit"},
            "Add": {"cwd", "sourceSet", "kind", "name", "operations", "dryRun"},
            "Edit": {"cwd", "sourceSet", "metadataPath", "operations", "dryRun"},
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
            # Общие для всех операций поля вставляются до match-выражения.
            properties = {"cwd", "sourceSet"}
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

        # The legacy operation union is published directly as
        # `properties.operations.items`, so a host that renders only
        # `properties` still sees the discriminated variants. It carries no
        # conditional composition and no owner-kind branching.
        edit_items = rust_function(metadata, "fn host_visible_operation_schema()")
        self.assertIn('"oneOf"', edit_items)
        for composition in ("anyOf", "allOf", '"if"', '"then"'):
            self.assertNotIn(composition, edit_items)
        schema_fn = rust_function(metadata, "pub fn metadata_input_schema")
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
            2,
        )
        success = rust_function(
            METADATA.read_text(encoding="utf-8"), "fn metadata_success"
        )
        self.assertIn("adapter: AdapterOutcome::ok(summary)", success)
        self.assertIn("data: Some(data)", success)

    def test_metadata_handlers_bypass_native_alias_and_descriptor_contracts(self) -> None:
        contracts = TOOL_CONTRACTS.read_text(encoding="utf-8")
        schema_path = rust_function(contracts, "pub fn input_schema_for_tool")
        shape_validation_path = rust_function(
            contracts, "pub fn validate_tool_argument_shape"
        )
        semantic_validation_path = rust_function(
            contracts, "pub fn validate_tool_argument_semantics"
        )
        self.assertIn("ToolHandler::Metadata", schema_path)
        self.assertIn("unica_metadata::request::metadata_input_schema(operation)", schema_path)
        self.assertIn("ToolHandler::Metadata", shape_validation_path)
        self.assertIn(
            "unica_metadata::request::validate_metadata_argument_shape(operation, args)",
            shape_validation_path,
        )
        self.assertIn("ToolHandler::Metadata", semantic_validation_path)
        self.assertIn(
            "unica_metadata::request::parse_metadata_request_after_shape(operation, args)",
            semantic_validation_path,
        )

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


for _retired_test in {
    "test_meta_info_complete_read_contract_is_synchronized",
    "test_meta_info_public_contract_distinguishes_empty_from_unavailable_memberships",
    "test_meta_observation_and_mutation_capability_contract_is_published",
}:
    setattr(
        MetaSurfaceContractTests,
        _retired_test,
        unittest.skip("retired v0.12 MCP surface; covered by the canonical v0.13 matrix")(
            getattr(MetaSurfaceContractTests, _retired_test)
        ),
    )


if __name__ == "__main__":
    unittest.main()
