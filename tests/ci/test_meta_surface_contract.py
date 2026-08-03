from __future__ import annotations

import re
import subprocess
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
APPLICATION = REPO_ROOT / "crates/unica-coder/src/application/mod.rs"
METADATA = REPO_ROOT / "crates/unica-coder/src/application/metadata.rs"
TOOL_CONTRACTS = REPO_ROOT / "crates/unica-coder/src/application/tool_contracts.rs"
OPERATION_DESCRIPTORS = (
    REPO_ROOT / "crates/unica-coder/src/application/operation_descriptors.rs"
)
META_RUNTIME = REPO_ROOT / "crates/unica-coder/src/infrastructure/native_operations/meta"
FORMAT_GUARD = REPO_ROOT / "crates/unica-coder/src/infrastructure/format_guard.rs"


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
    def test_live_meta_runtime_has_no_file_or_string_dsl_grammar(self) -> None:
        sources = [
            META_RUNTIME / "edit.rs",
            META_RUNTIME / "template_catalog.rs",
            META_RUNTIME / "xml_model.rs",
        ]
        retired_grammar = {
            "module-wide dead-code suppression": re.compile(
                r"^#!\[allow\([^\]]*dead_code", re.MULTILINE
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
        }

        violations: list[str] = []
        for path in sources:
            source = path.read_text(encoding="utf-8")
            for contract, pattern in retired_grammar.items():
                if pattern.search(source):
                    violations.append(f"{path.relative_to(REPO_ROOT)}: {contract}")
        format_guard = FORMAT_GUARD.read_text(encoding="utf-8")
        if '"meta-validate"' in format_guard:
            violations.append(
                f"{FORMAT_GUARD.relative_to(REPO_ROOT)}: retired meta-validate route"
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
            # Task 12 owns provenance reconciliation for these exact retained donors.
            "tests/fixtures/unica_mcp_script_parity/unica_reference_models/meta-compile/",
            "tests/fixtures/unica_mcp_script_parity/unica_reference_models/meta-edit/",
            "tests/fixtures/unica_mcp_script_parity/unica_reference_models/meta-validate/",
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
        for relative in tracked:
            if not relative or relative.startswith(excluded_prefixes):
                continue
            if relative == "tests/ci/test_meta_surface_contract.py":
                continue
            path = REPO_ROOT / relative
            if relative in retired_files and path.exists():
                violations.append(relative)
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

    def test_schema_path_has_exact_lower_camel_arguments_and_shallow_edit_items(self) -> None:
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
            "Add": {"sourceSet", "kind", "name", "dryRun"},
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

        edit_items = rust_function(metadata, "fn operation_schema()")
        self.assertEqual(
            set(re.findall(r'^ {12}"([a-zA-Z]+)":', edit_items, re.MULTILINE)),
            {
                "op",
                "values",
                "collection",
                "scope",
                "elements",
                "names",
                "relation",
                "mode",
                "targets",
            },
        )
        for composition in ("oneOf", "anyOf", "allOf"):
            self.assertNotIn(composition, edit_items)
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
        parser = rust_function(metadata, "pub(crate) fn parse_metadata_request")
        self.assertIn("(force && !confirm) || (confirm && !force)", parser)
        self.assertIn("dryRun=false applies it", parser)

        success = rust_function(metadata, "fn metadata_success")
        failure = rust_function(metadata, "fn metadata_failure")
        self.assertIn("adapter: AdapterOutcome::ok(summary)", success)
        self.assertIn("data: Some(data)", success)
        self.assertIn("stdout: None", failure)
        self.assertGreaterEqual(
            metadata.count("assert_eq!(outcome.adapter.stdout, None);"),
            4,
            "public Meta coordinator tests must prove data-only results",
        )

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
