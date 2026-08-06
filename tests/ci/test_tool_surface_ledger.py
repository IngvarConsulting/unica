"""The tool-surface ledger must describe the registry that actually ships.

A hand-maintained inventory of 71 tools drifts on the first merge, so the
mechanical columns are generated and this guard fails when they stop matching
the built binary or when a tool has no review entry.
"""

from __future__ import annotations

import collections
import importlib.util
import json
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
GENERATOR = REPO_ROOT / "scripts/ci/generate-tool-surface.py"
LEDGER = REPO_ROOT / "spec/architecture/tool-surface.md"
REVIEW = REPO_ROOT / "spec/architecture/tool-surface-review.json"
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

    def test_schema_base_resolution_is_bounded(self) -> None:
        root = {"$defs": {"cycle": {"$ref": "#/$defs/cycle"}}}
        with self.assertRaisesRegex(AssertionError, "resolution exceeded"):
            resolve_schema_base(root, root["$defs"]["cycle"])

    def test_xdto_group_has_a_human_domain_title(self) -> None:
        self.assertEqual(self.module.GROUP_TITLES.get("xdto"), "xdto — пакеты XDTO")

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


if __name__ == "__main__":
    unittest.main()
