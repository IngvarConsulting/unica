"""The generated ledger must describe the canonical v0.13 surface that ships."""

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
LEDGER = REPO_ROOT / "arch/tool-surface.md"
REVIEW = REPO_ROOT / "arch/tool-surface-review.json"
RESULT_CONTRACT_INVARIANT = (
    REPO_ROOT / "arch/invariants/INV.SURFACE.RESULT-CONTRACTS-MATCH-REVIEW.md"
)
RUN_COVERAGE = REPO_ROOT / "arch/tool-implementation-coverage.json"
# A run operation name on the wire: two or more dotted lowercase segments.
# Tool names share the shape and are told apart by their `unica.` head.
OPERATION_TOKEN = re.compile(r"\b[a-z][a-z0-9]*(?:\.[a-z][a-z0-9]*)+\b")
# Счёт в плане написан словом, и сравнивать его надо словом же. Таблица
# закрыта размером словаря: больше его операций не бывает.
RUSSIAN_NUMERALS = {
    0: "ноль",
    1: "одну",
    2: "две",
    3: "три",
    4: "четыре",
    5: "пять",
    6: "шесть",
    7: "семь",
    8: "восемь",
    9: "девять",
    10: "десять",
}
BINARY = REPO_ROOT / "target/debug/unica"

NATIVE_V13 = [
    "unica.view",
    "unica.apply",
    "unica.resolve",
    "unica.search",
    "unica.check",
    "unica.diff",
    "unica.run",
    "unica.docs",
]
TASK_COMPATIBILITY = ["unica.task.get", "unica.task.result", "unica.task.cancel"]


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
                {"required": ["sourceSet"], "not": {"required": ["SubsystemPath"]}},
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
        rendered = "\n".join(self.module.render_arguments({"inputSchema": schema}))

        def row(argument: str) -> str:
            return next(
                line
                for line in rendered.splitlines()
                if line.startswith(f"| `{argument}`")
            )

        self.assertIn(" только в ветви |", row("metadataPath"))
        self.assertIn(" по ветви |", row("sourceSet"))
        self.assertIn(" по ветви |", row("SubsystemPath"))
        self.assertIn(" нет |", row("cwd"))

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
            ],
        }
        rendered = "\n".join(self.module.render_arguments({"inputSchema": schema}))
        self.assertIn("| `action` | string | да |", rendered)
        self.assertIn("| `sourceSet` | string | да |", rendered)
        self.assertIn("| `metadataPath` | string | по ветви |", rendered)
        self.assertIn("| `timeoutSeconds` | integer | только в ветви |", rendered)

    def test_schema_base_resolution_is_bounded(self) -> None:
        root = {"$defs": {"cycle": {"$ref": "#/$defs/cycle"}}}
        with self.assertRaisesRegex(AssertionError, "resolution exceeded"):
            resolve_schema_base(root, root["$defs"]["cycle"])

    def test_published_patterns_stay_inside_the_ecmascript_dialect(self) -> None:
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

    def test_registry_is_exactly_the_v13_compatibility_surface(self) -> None:
        names = [tool["name"] for tool in self.tools]
        self.assertEqual(names, NATIVE_V13 + TASK_COMPATIBILITY)
        self.assertEqual(len(names), len(set(names)), "duplicate public tool definition")
        self.assertFalse(
            set(names)
            & {
                "unica.project.status",
                "unica.standards.search",
                "unica.standards.explain",
            }
        )

    def test_canonical_subject_schemas_have_only_logical_inputs(self) -> None:
        tools = {tool["name"]: tool for tool in self.tools}
        expected_properties = {
            "unica.view": {"at", "filter", "limit", "cursor"},
            "unica.apply": {"at", "ops", "dryRun", "ifRev"},
            # Единственный инструмент, которому путь на входе разрешён:
            # аварийный мост затем и заведён, чтобы путь не просачивался
            # в частые ответы.
            "unica.resolve": {"at", "path"},
            # `corpus` выбирает свод — текст модулей или имена метаданных, —
            # а `kind` сужает поиск по именам до одного вида узла. Оба входа
            # логические: ни один не называет файл.
            "unica.search": {"query", "corpus", "kind", "role", "scope", "regex", "limit"},
            "unica.check": {"at"},
            "unica.diff": {"left", "right", "filter", "limit", "cursor"},
            "unica.run": {"op", "args", "dryRun", "ifRev"},
            "unica.docs": {"query", "source"},
        }
        for name, properties in expected_properties.items():
            with self.subTest(tool=name):
                schema = tools[name]["inputSchema"]
                self.assertEqual(schema["type"], "object")
                self.assertFalse(schema["additionalProperties"])
                self.assertEqual(set(schema["properties"]), properties)
                encoded = json.dumps(schema, ensure_ascii=False)
                # Аварийный мост — единственное место, где путь законен на
                # входе: он затем и заведён, чтобы путь не просачивался в
                # частые ответы (DEC.2026-09-08.RESOLVE-REPLACES-FIND).
                physical_inputs = ("cwd", "sourceDir", "workdir")
                if name != "unica.resolve":
                    physical_inputs += ("path",)
                for physical in physical_inputs:
                    self.assertNotIn(f'"{physical}"', encoded)

    def test_every_published_tool_has_exactly_one_review_entry(self) -> None:
        names = [tool["name"] for tool in self.tools]
        self.assertEqual(set(names), set(self.review))
        self.assertEqual(len(names), len(self.review))

    def test_every_review_entry_states_a_typed_in_scope_contract(self) -> None:
        for name, entry in sorted(self.review.items()):
            with self.subTest(tool=name):
                self.assertEqual(entry["scope"], "in")
                self.assertEqual(entry["result"]["contract"], "typed")
                self.assertTrue(entry["result"]["now"].strip())
                self.assertTrue(entry["result"]["target"].strip())
                self.assertGreaterEqual(len(entry["scenarios"]), 1)
                self.assertTrue(all(scenario.strip() for scenario in entry["scenarios"]))

    def test_the_published_run_prose_counts_the_dictionary_it_describes(self) -> None:
        """Счёт в прозе поверхности держится на словаре, а не на памяти.

        Имя операции в `now`, `target` и сценариях читают как адрес вызова:
        назвать операцию вне словаря — отправить читателя в отказ. Счёт
        нереализованных операций в плане устаревает так же молча, как счёт в
        прозе правила (#798), и его надо считать, а не помнить. Словарь ведёт
        `arch/tool-implementation-coverage.json`.
        """
        operations = json.loads(RUN_COVERAGE.read_text(encoding="utf-8"))[
            "runOperations"
        ]
        entry = self.review["unica.run"]
        prose = [entry["result"]["now"], entry["result"]["target"], *entry["scenarios"]]
        for text in prose:
            for token in OPERATION_TOKEN.findall(text):
                if token.startswith("unica."):
                    continue
                with self.subTest(token=token):
                    self.assertIn(token, operations)

        unimplemented = sum(
            1 for entry in operations.values() if entry["status"] != "supported"
        )
        self.assertIn(
            RUSSIAN_NUMERALS[unimplemented],
            entry["result"]["target"],
            "план называет счёт нереализованных операций словом",
        )

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
        self.assertIn("в границах работы: **0**", text)

    def test_typed_result_invariant_names_the_registry_contract_check(self) -> None:
        text = RESULT_CONTRACT_INVARIANT.read_text(encoding="utf-8")
        self.assertIn("id: INV.SURFACE.RESULT-CONTRACTS-MATCH-REVIEW", text)
        self.assertIn("decision: DEC.2026-08-18.CARRIED-RULES", text)
        self.assertIn(
            "check: crates/unica-coder/src/application/mod.rs::tool_specs_match_reviewed_result_contracts",
            text,
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
