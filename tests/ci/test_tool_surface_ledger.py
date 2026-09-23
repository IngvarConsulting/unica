"""The generated ledger must describe the canonical v0.13 surface that ships."""

from __future__ import annotations

import collections
import importlib.util
import json
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
GENERATOR = REPO_ROOT / "scripts/ci/generate-tool-surface.py"
LEDGER = REPO_ROOT / "docs/tool-surface.md"
REVIEW = REPO_ROOT / "tests/fixtures/v013/tool-surface-review.json"
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
BOOTSTRAP_VERIFICATION = REPO_ROOT / "crates/unica-bootstrap/src/verification.rs"
LEDGER_TOOL_HEADING = re.compile(r"^### `(unica\.[a-z0-9.]+)`$", re.M)


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
            "unica.search": {"query", "corpus", "kind", "role", "scope", "regex", "limit", "cursor"},
            "unica.check": {"at"},
            "unica.diff": {"left", "right", "filter", "limit", "cursor"},
            "unica.run": {"op", "args", "dryRun", "ifRev", "infobase"},
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

    def test_published_run_operation_names_belong_to_the_dictionary(self) -> None:
        """JSON-примеры называют реализованные операции публичного словаря."""
        from tests.ci.test_acceptance_scenarios import AcceptanceServer
        from tests.ci.test_unica_skills import collect_runtime_guidance

        plugin = REPO_ROOT / "plugins/unica"
        documents = sorted(
            list((plugin / "skills").rglob("*.md"))
            + list((plugin / "references").rglob("*.md"))
        )
        _, examples, failures = collect_runtime_guidance(
            [(document, document.read_text(encoding="utf-8")) for document in documents]
        )
        self.assertEqual(failures, [])
        self.assertTrue(examples, "no published runtime JSON examples were checked")

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            workspace = root / "workspace"
            state = root / "state"
            workspace.mkdir()
            state.mkdir()
            server = AcceptanceServer(workspace, state, "2025-11-25")
            try:
                response = server.call("unica.run", {})
            finally:
                server.close()
                server.reader.join(timeout=10)
                server.process.stdout.close()

        self.assertIsNotNone(response)
        result = response["result"]["structuredContent"]
        self.assertTrue(result["ok"], result)
        operations = {operation["op"]: operation for operation in result["data"]["operations"]}
        for document, arguments in examples:
            operation = arguments.get("op")
            with self.subTest(path=document.relative_to(REPO_ROOT), operation=operation):
                self.assertIn(operation, operations)
                self.assertIs(operations[operation]["implemented"], True)

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


class SurfaceCopiesAgreeTests(unittest.TestCase):
    """Копии имён поверхности вне check контракта обязаны совпадать с ведомостью.

    Ведомость порождается из бинаря, и её сверяет test_ledger_matches_the_live_registry.
    Остальные списки — ожидание этого теста и константа бутстрапа — не
    проверки контракта, а его потребители; расхождение обязано ломаться здесь,
    одним сообщением, называющим оба места (#699).
    """

    def test_ledger_names_the_expected_compatibility_surface(self) -> None:
        ledger = LEDGER_TOOL_HEADING.findall(LEDGER.read_text(encoding="utf-8"))
        self.assertEqual(sorted(ledger), sorted(NATIVE_V13 + TASK_COMPATIBILITY), str(LEDGER))
        self.assertEqual(len(ledger), len(set(ledger)), f"дубли заголовков в {LEDGER}")

    def test_bootstrap_verification_copy_matches_the_ledger(self) -> None:
        source = BOOTSTRAP_VERIFICATION.read_text(encoding="utf-8")
        block = re.search(
            r"EXPECTED_COMPATIBILITY_TOOLS: \[&str; (\d+)\] = \[(.*?)\];", source, re.S
        )
        self.assertIsNotNone(block, f"нет EXPECTED_COMPATIBILITY_TOOLS в {BOOTSTRAP_VERIFICATION}")
        names = re.findall(r'"(unica\.[a-z0-9.]+)"', block.group(2))
        self.assertEqual(int(block.group(1)), len(names))
        ledger = LEDGER_TOOL_HEADING.findall(LEDGER.read_text(encoding="utf-8"))
        self.assertEqual(
            sorted(names),
            sorted(ledger),
            f"{BOOTSTRAP_VERIFICATION} расходится с {LEDGER}",
        )


if __name__ == "__main__":
    unittest.main()
