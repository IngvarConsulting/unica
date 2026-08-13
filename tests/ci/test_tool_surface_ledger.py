"""The tool-surface ledger must describe the registry that actually ships.

A hand-maintained tool inventory drifts on the first merge, so the
mechanical columns are generated and this guard fails when they stop matching
the built binary or when a tool has no review entry.
"""

from __future__ import annotations

import collections
import importlib.util
import json
import re
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
GENERATOR = REPO_ROOT / "scripts/ci/generate-tool-surface.py"
LEDGER = REPO_ROOT / "spec/architecture/tool-surface.md"
REVIEW = REPO_ROOT / "spec/architecture/tool-surface-review.json"
INVARIANTS = REPO_ROOT / "spec/architecture/invariants.md"
BINARY = REPO_ROOT / "target/debug/unica"


def load_generator():
    spec = importlib.util.spec_from_file_location("unica_tool_surface", GENERATOR)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def resolve_schema_base(root, schema):
    """Resolve root-local definitions and the shared base of an allOf profile."""
    resolution_limit = len(root.get("$defs", {})) + 2
    for _ in range(resolution_limit):
        reference = schema.get("$ref")
        if reference is not None:
            assert reference.startswith("#/$defs/")
            schema = root["$defs"][reference.removeprefix("#/$defs/")]
            continue
        all_of = schema.get("allOf")
        if all_of:
            schema = all_of[0]
            continue
        return schema
    raise AssertionError(
        f"schema base resolution exceeded {resolution_limit} local steps"
    )


class ToolSurfaceLedgerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        subprocess.run(
            ["cargo", "build", "--quiet", "--package", "unica-coder", "--bin", "unica"],
            cwd=REPO_ROOT,
            check=True,
        )
        cls.module = load_generator()
        cls.tools = cls.module.read_registry(BINARY)
        cls.review = json.loads(REVIEW.read_text(encoding="utf-8"))

    def test_a_branch_only_argument_is_not_published_as_freely_optional(self) -> None:
        """ADR-0049: `metadataPath` is valid only alongside `sourceSet`.

        `unica.subsystem.info` forbids it in the path branch, so rendering it
        as plain `нет` tells a caller it may be sent with `SubsystemPath`.
        """
        schema = {
            "type": "object",
            "properties": {
                "SubsystemPath": {"type": "string"},
                "sourceSet": {"type": "string"},
                "metadataPath": {"type": "string"},
                "cwd": {"type": "string"},
            },
            "required": [],
            "oneOf": [
                {
                    "required": ["sourceSet"],
                    "not": {"required": ["SubsystemPath"]},
                },
                {
                    "required": ["SubsystemPath"],
                    "not": {
                        "anyOf": [
                            {"required": ["sourceSet"]},
                            {"required": ["metadataPath"]},
                        ]
                    },
                },
            ],
        }
        rendered = "\n".join(
            self.module.render_arguments({"inputSchema": schema})
        )

        def row(argument: str) -> str:
            return next(
                line
                for line in rendered.splitlines()
                if line.startswith(f"| `{argument}`")
            )

        # Assert the exact marker, not merely "not optional": `да` and
        # `по ветви` are both wrong here and both would pass a negative check.
        # `metadataPath` is valid only inside the sourceSet branch, while
        # `sourceSet` is required by one branch and refused by the other.
        self.assertIn(" только в ветви |", row("metadataPath"), row("metadataPath"))
        self.assertIn(" по ветви |", row("sourceSet"), row("sourceSet"))
        self.assertIn(" по ветви |", row("SubsystemPath"), row("SubsystemPath"))
        # `cwd` is genuinely optional in both branches and must stay that way.
        self.assertIn(" нет |", row("cwd"), row("cwd"))
        self.assertIn(
            "`metadataPath` принимается только вместе с `sourceSet`.",
            rendered,
            rendered,
        )

    def test_discriminated_object_branches_render_their_argument_union(self) -> None:
        schema = {
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "action": {"type": "string"},
                "sourceSet": {"type": "string"},
                "timeoutSeconds": {"type": "integer"},
                "metadataPath": {"type": "string"},
            },
            "required": [],
            "oneOf": [
                {
                    "type": "object",
                    "additionalProperties": False,
                    "properties": {
                        "action": {"type": "string", "const": "analyze"},
                        "sourceSet": {"type": "string"},
                        "timeoutSeconds": {"type": "integer"},
                    },
                    "required": ["action", "sourceSet"],
                },
                {
                    "type": "object",
                    "additionalProperties": False,
                    "properties": {
                        "action": {"type": "string", "const": "findings"},
                        "sourceSet": {"type": "string"},
                        "metadataPath": {"type": "string"},
                    },
                    "required": ["action", "sourceSet", "metadataPath"],
                },
            ]
        }

        rendered = "\n".join(
            self.module.render_arguments({"inputSchema": schema})
        )

        self.assertIn("| `action` | string | да |", rendered)
        self.assertIn("| `sourceSet` | string | да |", rendered)
        self.assertIn("| `metadataPath` | string | по ветви |", rendered)
        self.assertIn("| `timeoutSeconds` | integer | только в ветви |", rendered)
        self.assertNotIn("Опубликованных аргументов нет", rendered)

    def test_published_patterns_stay_inside_the_ecmascript_dialect(self) -> None:
        """JSON Schema `pattern` — это ECMA-262.

        Без флага `u` конструкция `\\p{...}` там значит литеральную `p`, а не
        класс символов, поэтому хост молча примет неверный вход или отвергнет
        верный. Юникодные классы остаются во внутренних Rust-проверках.
        """

        offenders = []

        def walk(node: object, path: str) -> None:
            if isinstance(node, dict):
                for key, value in node.items():
                    if key == "pattern" and isinstance(value, str) and "\\p{" in value:
                        offenders.append(f"{path}: {value}")
                    walk(value, f"{path}.{key}")
            elif isinstance(node, list):
                for index, value in enumerate(node):
                    walk(value, f"{path}[{index}]")

        for tool in self.tools:
            walk(tool.get("inputSchema"), f"{tool['name']}.inputSchema")
            walk(tool.get("outputSchema"), f"{tool['name']}.outputSchema")

        self.assertEqual(offenders, [])

    def test_every_published_tool_has_a_review_entry(self) -> None:
        published = {tool["name"] for tool in self.tools}
        self.assertEqual(published - set(self.review), set())
        self.assertEqual(set(self.review) - published, set())

    def test_xdto_surface_is_exactly_the_typed_info_edit_pair(self) -> None:
        expected = {"unica.xdto.info", "unica.xdto.edit"}
        published = {
            tool["name"] for tool in self.tools if tool["name"].startswith("unica.xdto.")
        }
        reviewed = {name for name in self.review if name.startswith("unica.xdto.")}

        self.assertEqual(published, expected)
        self.assertEqual(reviewed, expected)
        for name in sorted(expected):
            with self.subTest(tool=name):
                self.assertEqual(self.review[name]["scope"], "in")
                self.assertEqual(self.review[name]["result"]["contract"], "typed")

    def test_diagnostics_surface_is_logical_provider_neutral_and_clean_break(self) -> None:
        tool = next(tool for tool in self.tools if tool["name"] == "unica.code.diagnostics")
        review = self.review["unica.code.diagnostics"]

        self.assertEqual(review["scope"], "in")
        self.assertEqual(review["result"]["contract"], "typed")
        for token in ("provider", "location", "focus", "sourceSet", "metadataPath"):
            self.assertIn(token, review["result"]["now"])

        branches = tool["inputSchema"]["oneOf"]
        self.assertEqual(
            {branch["properties"]["action"]["const"] for branch in branches},
            {"analyze", "findings", "status", "catalog"},
        )
        all_arguments = {
            argument for branch in branches for argument in branch["properties"]
        }
        for required in ("action", "sourceSet", "providers", "filter"):
            self.assertIn(required, all_arguments)
        self.assertIn("range", all_arguments)
        for legacy in ("mode", "sourceDir", "path", "codes", "rangeStart", "rangeEnd"):
            self.assertNotIn(legacy, all_arguments)

    def test_schema_base_resolution_is_bounded(self) -> None:
        root = {"$defs": {"cycle": {"$ref": "#/$defs/cycle"}}}
        with self.assertRaisesRegex(AssertionError, "resolution exceeded"):
            resolve_schema_base(root, root["$defs"]["cycle"])

    def test_xdto_group_has_a_human_domain_title(self) -> None:
        self.assertEqual(self.module.GROUP_TITLES.get("xdto"), "xdto — пакеты XDTO")

    def test_role_surface_contains_one_logical_typed_edit_contract(self) -> None:
        expected = {
            "unica.role.compile",
            "unica.role.edit",
            "unica.role.info",
            "unica.role.validate",
        }
        published = {
            tool["name"]: tool
            for tool in self.tools
            if tool["name"].startswith("unica.role.")
        }
        reviewed = {name for name in self.review if name.startswith("unica.role.")}

        self.assertEqual(set(published), expected)
        self.assertEqual(reviewed, expected)
        review = self.review["unica.role.edit"]
        self.assertEqual(review["scope"], "in")
        self.assertEqual(review["result"]["contract"], "typed")
        for token in (
            "metadataPath",
            "changed",
            "effects",
            "operationIndex",
            "validation",
            "diagnostics",
        ):
            with self.subTest(result_token=token):
                self.assertIn(token, review["result"]["now"])

        schema = published["unica.role.edit"]["inputSchema"]
        self.assertEqual(schema["type"], "object")
        self.assertFalse(schema["additionalProperties"])
        self.assertEqual(
            set(schema["properties"]),
            {"sourceSet", "metadataPath", "operations", "dryRun"},
        )
        self.assertEqual(
            schema["required"], ["sourceSet", "metadataPath", "operations"]
        )
        self.assertIs(schema["properties"]["dryRun"]["default"], True)
        self.assertEqual(
            schema["properties"]["metadataPath"]["pattern"],
            r"^Role\.[^.]+$",
        )

        operations = schema["properties"]["operations"]
        self.assertEqual(operations["type"], "array")
        self.assertEqual(operations["minItems"], 1)
        operation = operations["items"]
        self.assertEqual(operation["type"], "object")
        self.assertFalse(operation["additionalProperties"])
        self.assertEqual(
            set(operation["properties"]), {"op", "objectName", "right", "value"}
        )
        self.assertEqual(
            operation["required"], ["op", "objectName", "right", "value"]
        )
        self.assertEqual(operation["properties"]["op"], {"const": "setRight"})
        self.assertEqual(operation["properties"]["value"]["type"], "boolean")
        self.assertTrue(operation["properties"]["right"]["enum"])

        encoded = json.dumps(schema, ensure_ascii=False)
        for legacy in ("RightsPath", "Path", "ObjectName", "Name", "Value"):
            with self.subTest(legacy=legacy):
                self.assertNotIn(f'"{legacy}"', encoded)

        output = published["unica.role.edit"]["outputSchema"]
        self.assertEqual(output["type"], "object")
        self.assertFalse(output["additionalProperties"])
        data = output["properties"]["data"]
        self.assertEqual(data["type"], "object")
        self.assertFalse(data["additionalProperties"])
        self.assertEqual(
            set(data["properties"]),
            {"metadataPath", "changed", "effects", "validation", "diagnostics"},
        )
        self.assertEqual(
            data["required"],
            ["metadataPath", "changed", "effects", "validation", "diagnostics"],
        )
        effect = data["properties"]["effects"]["items"]
        self.assertFalse(effect["additionalProperties"])
        self.assertEqual(effect["properties"]["operation"], {"const": "setRight"})
        self.assertEqual(
            effect["properties"]["action"]["enum"], ["setRight", "removeObject"]
        )
        self.assertEqual(
            data["properties"]["validation"]["properties"]["status"]["enum"],
            ["passed", "failed"],
        )

    def test_meta_surface_is_exactly_four_typed_operation_contracts(self) -> None:
        expected = {
            "unica.meta.info",
            "unica.meta.add",
            "unica.meta.edit",
            "unica.meta.remove",
        }
        published = {
            tool["name"]: tool
            for tool in self.tools
            if tool["name"].startswith("unica.meta.")
        }
        reviewed = {name for name in self.review if name.startswith("unica.meta.")}

        self.assertEqual(set(published), expected)
        self.assertEqual(reviewed, expected)
        for name in sorted(expected):
            with self.subTest(tool=name):
                self.assertEqual(self.review[name]["scope"], "in")
                self.assertEqual(self.review[name]["result"]["contract"], "typed")

        operation_schemas = {
            name: published[name]["inputSchema"]["properties"]["operations"]
            for name in ("unica.meta.add", "unica.meta.edit")
        }
        for name, operations in operation_schemas.items():
            with self.subTest(tool=name, field="operations"):
                self.assertEqual(operations["type"], "array")
                self.assertEqual(operations["minItems"], 1)
                # A host that renders only `properties` never evaluates a
                # conditional and may not resolve `$ref`. Without a direct
                # `items` such a host offers the model an untyped array, so the
                # kind-agnostic union ships inline (ADR-0025).
                items = operations["items"]
                branches = items["oneOf"]
                self.assertEqual(
                    {branch["properties"]["op"]["enum"][0] for branch in branches},
                    {"setProperties", "add", "update", "remove", "editRelations"},
                )
                for branch in branches:
                    self.assertNotIn("$ref", branch)
                    self.assertIn("op", branch["required"])

        add_root = published["unica.meta.add"]["inputSchema"]
        edit_root = published["unica.meta.edit"]["inputSchema"]
        # ADR-0025: the union is the whole published contract. No conditional
        # branches remain, and both mutations publish the same union and the
        # same shared definitions.
        self.assertNotIn("allOf", add_root)
        self.assertNotIn("allOf", edit_root)
        self.assertEqual(add_root["$defs"], edit_root["$defs"])
        self.assertEqual(
            sorted(add_root["$defs"]),
            ["fillValue", "metadataType", "position", "scope"],
        )
        self.assertEqual(
            add_root["properties"]["operations"]["items"],
            edit_root["properties"]["operations"]["items"],
        )
        variants = edit_root["properties"]["operations"]["items"]["oneOf"]
        for variant in variants:
            self.assertEqual(variant["type"], "object")
            self.assertFalse(variant["additionalProperties"])
            self.assertIn("op", variant["required"])
        self.assertEqual(
            {variant["properties"]["op"]["enum"][0] for variant in variants},
            {"setProperties", "add", "update", "remove", "editRelations"},
        )
        # The union publishes closed domains, not a bare name list: a model that
        # reads only the schema must learn the legal values too.
        values = next(
            variant
            for variant in variants
            if variant["properties"]["op"]["enum"][0] == "setProperties"
        )["properties"]["values"]
        self.assertFalse(values["additionalProperties"])
        self.assertEqual(
            values["properties"]["HierarchyType"]["enum"],
            ["HierarchyFoldersAndItems", "HierarchyOfItems"],
        )

        self.assertNotIn(
            "upsert-predefined",
            json.dumps(operation_schemas, ensure_ascii=False),
        )
        predefined = [
            variant
            for variant in variants
            if variant["properties"].get("collection", {}).get("enum")
            == ["predefinedItems"]
        ]
        self.assertEqual(len(predefined), 3)
        self.assertEqual(
            {variant["properties"]["op"]["enum"][0] for variant in predefined},
            {"add", "update", "remove"},
        )
        expected_item_fields = {
            "id",
            "name",
            "code",
            "description",
            "isFolder",
            "type",
            "accountType",
            "offBalance",
            "order",
            "accountingFlags",
            "extDimensionTypes",
            "actionPeriodIsBase",
        }
        for variant in predefined:
            tag = variant["properties"]["op"]["enum"][0]
            with self.subTest(predefined_operation=tag):
                self.assertFalse(variant["additionalProperties"])
                self.assertNotIn("scope", variant["properties"])
                self.assertNotIn("names", variant["properties"])
                if tag == "remove":
                    self.assertEqual(
                        set(variant["properties"]), {"op", "collection", "ids"}
                    )
                    self.assertEqual(
                        variant["required"], ["op", "collection", "ids"]
                    )
                    ids = variant["properties"]["ids"]
                    self.assertEqual(ids["items"]["format"], "uuid")
                    self.assertTrue(ids["uniqueItems"])
                    continue

                self.assertEqual(
                    set(variant["properties"]), {"op", "collection", "elements"}
                )
                self.assertEqual(
                    variant["required"], ["op", "collection", "elements"]
                )
                item = variant["properties"]["elements"]["items"]
                self.assertFalse(item["additionalProperties"])
                self.assertEqual(set(item["properties"]), expected_item_fields)
                self.assertEqual(item["properties"]["id"]["format"], "uuid")
                if tag == "add":
                    self.assertEqual(item["required"], ["id", "name"])
                else:
                    self.assertEqual(item["required"], ["id"])
                    self.assertEqual(item["minProperties"], 2)

        generic_collection_branches = [
            variant
            for variant in variants
            if "collection" in variant["properties"] and variant not in predefined
        ]
        self.assertTrue(generic_collection_branches)
        for variant in generic_collection_branches:
            self.assertNotIn(
                "predefinedItems", variant["properties"]["collection"]["enum"]
            )

    def test_every_review_entry_states_a_contract_and_scenarios(self) -> None:
        for name, entry in sorted(self.review.items()):
            with self.subTest(tool=name):
                # The migration metric reads this field, never the free-text
                # note beside it: counting progress by matching substrings in
                # prose is the very mistake ADR-0023 removes from the tools.
                self.assertIn(entry["result"]["contract"], self.module.CONTRACT_STATES)
                self.assertIn(entry["scope"], self.module.SCOPE_TITLES)
                self.assertTrue(entry["result"]["now"].strip(), name)
                self.assertTrue(entry["result"]["target"].strip(), name)
                # One scenario documents a tool nobody reviewed against real
                # use; the point of the ledger is more than a restated summary.
                self.assertGreaterEqual(len(entry["scenarios"]), 1, name)
                for scenario in entry["scenarios"]:
                    self.assertTrue(scenario.strip(), name)

    def test_ledger_matches_the_live_registry(self) -> None:
        result = subprocess.run(
            [sys.executable, str(GENERATOR), "--check", "--binary", str(BINARY)],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)

    def test_ledger_counts_the_migration_it_tracks(self) -> None:
        text = LEDGER.read_text(encoding="utf-8")
        self.assertIn(f"- Инструментов: **{len(self.tools)}**", text)
        states = collections.Counter(
            entry["result"]["contract"] for entry in self.review.values()
        )
        self.assertEqual(sum(states.values()), len(self.tools))
        for state, title in self.module.CONTRACT_STATES.items():
            self.assertIn(f"- {title}: **{states[state]}**", text)
        remaining = sum(
            1
            for entry in self.review.values()
            if entry["scope"] == "in" and entry["result"]["contract"] != "typed"
        )
        self.assertIn(f"в границах работы: **{remaining}**", text)

    def test_retiring_and_runtime_tools_stay_outside_the_typing_work(self) -> None:
        """A tool slated for removal is not worth a new contract, and the
        runtime family is decided separately: both must be excluded by an
        explicit field, not by whoever remembers the conversation."""

        for name, entry in sorted(self.review.items()):
            operation = name.rsplit(".", 1)[-1]
            with self.subTest(tool=name):
                if operation in {"validate", "compile", "decompile"}:
                    self.assertEqual(entry["scope"], "retiring")
                elif name.startswith(("unica.runtime.", "unica.build.")):
                    self.assertEqual(entry["scope"], "runtime")
                else:
                    self.assertEqual(entry["scope"], "in")

    def test_a_partially_typed_tool_is_not_counted_as_migrated(self) -> None:
        """A tool that answers with some `data` and some prose is not migrated.
        Counting it as done hides a tool that still has to move, which is how
        meta.info stayed invisible until its contract was stated explicitly."""

        partial = {
            name
            for name, entry in self.review.items()
            if entry["result"]["contract"] == "partial"
        }
        self.assertTrue(
            partial,
            "the ledger must keep naming partially typed tools while any remain",
        )
        # The count comes from the contract field alone. The original defect
        # counted a tool as migrated because the word `data` appeared in its
        # free-text note, which is how meta.info escaped the number.
        typed = {
            name
            for name, entry in self.review.items()
            if entry["result"]["contract"] == "typed"
        }
        self.assertEqual(partial & typed, set())
        text = LEDGER.read_text(encoding="utf-8")
        self.assertIn(
            f"- {self.module.CONTRACT_STATES['typed']}: **{len(typed)}**", text
        )
        self.assertIn(
            f"- {self.module.CONTRACT_STATES['partial']}: **{len(partial)}**", text
        )

    def test_typed_result_invariant_names_the_registry_contract_check(self) -> None:
        text = INVARIANTS.read_text(encoding="utf-8")
        section = text.split("### INV-MCP-TYPED-RESULT", 1)[1].split("\n### ", 1)[0]
        fields: dict[str, list[str]] = collections.defaultdict(list)
        current_field: str | None = None
        for line in section.splitlines():
            match = re.match(r"- \*\*(Rule|Decision|Check):\*\*\s*(.*)", line)
            if match:
                current_field = match.group(1)
                fields[current_field].append(match.group(2))
            elif current_field is not None and line.startswith("  "):
                fields[current_field][-1] += " " + line.strip()
            else:
                current_field = None

        self.assertIn("typed_result_missing", " ".join(fields["Rule"]))
        self.assertIn("ADR-0044", " ".join(fields["Decision"]))
        self.assertIn(
            "`crates/unica-coder/src/application/mod.rs`",
            " ".join(fields["Check"]),
        )
        application_tests = (
            REPO_ROOT / "crates/unica-coder/src/application/mod.rs"
        ).read_text(encoding="utf-8")
        self.assertRegex(
            application_tests,
            r"fn\s+tool_specs_match_reviewed_result_contracts\s*\(",
        )


if __name__ == "__main__":
    unittest.main()
