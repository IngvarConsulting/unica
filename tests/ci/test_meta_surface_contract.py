from __future__ import annotations

import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
APPLICATION = REPO_ROOT / "crates/unica-coder/src/application/mod.rs"
METADATA = REPO_ROOT / "crates/unica-coder/src/application/metadata.rs"
TOOL_CONTRACTS = REPO_ROOT / "crates/unica-coder/src/application/tool_contracts.rs"
OPERATION_DESCRIPTORS = (
    REPO_ROOT / "crates/unica-coder/src/application/operation_descriptors.rs"
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
