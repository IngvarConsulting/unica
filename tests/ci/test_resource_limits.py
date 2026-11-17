from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("resource_limits", ROOT / "scripts/ci/check-resource-limits.py")
limits = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(limits)


def block(ident: str = "demo.file-bytes", indent: str = "", **overrides: str | list[str] | None) -> str:
    """A `// resource-limit:` block; a field set to `None` is left out."""
    fields: dict[str, str | list[str] | None] = {
        "class": "corpus",
        "unit": "bytes",
        "scope": "file",
        "value": str(8 * 1024 * 1024),
        "derived": None,
        "literal": None,
        "count": None,
        "expr": None,
        "resource": "байты одного файла",
        "status": "unproven",
        "origin": "неизвестно",
        "problem": "неизвестно",
        "on-reach": "отказ source_unreadable",
        "issue": "#1119",
        "evidence": [],
        "decision": None,
        "decision-source": None,
        "review": None,
    }
    fields.update({name.replace("_", "-"): value for name, value in overrides.items()})
    lines = [f"// resource-limit: {ident}"]
    for name, value in fields.items():
        for item in value if isinstance(value, list) else [value]:
            if item is not None:
                lines.append(f"// {name}: {item}")
    return "".join(f"{indent}{line}\n" for line in lines)


TIMEOUT = block(
    "demo.read-timeout",
    **{"class": "deadline", "unit": "ms", "scope": "request", "value": "30000", "issue": "#1312", "on_reach": "отказ deadline_exceeded"},
)
LITERAL = block(
    "demo.literal-bytes",
    indent="    ",
    value=str(32 * 1024 * 1024),
    literal="vec![0u8; 32 * 1024 * 1024]",
    count="1",
    expr="32 * 1024 * 1024",
)


class ResourceLimitBlockTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.lib(block())
        self.write("crates/demo/src/read.rs", "use crate::MAX_FILE_BYTES;\n")
        self.write("crates/demo/tests/large.rs", "const MAX_FIXTURE_BYTES: usize = 9;\nuse demo::MAX_FILE_BYTES;\n")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write(self, relative: str, text: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def lib(self, file_block: str, timeout: str = TIMEOUT, literal: str = LITERAL) -> None:
        self.write(
            "crates/demo/src/lib.rs",
            "/// The bytes of one file.\n"
            f"{file_block}pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;\n"
            f"{timeout}const READ_TIMEOUT: Duration = Duration::from_secs(30);\n"
            f"fn read() {{\n{literal}    let _ = vec![0u8; 32 * 1024 * 1024];\n}}\n"
            "#[cfg(test)]\nmod tests {\n    const MAX_TEST_BYTES: usize = 1;\n}\n",
        )

    def line_of(self, text: str) -> str:
        lines = (self.root / "crates/demo/src/lib.rs").read_text(encoding="utf-8").splitlines()
        return f"crates/demo/src/lib.rs:{next(index for index, line in enumerate(lines, 1) if text in line)}"

    def errors(self) -> list[str]:
        return limits.check(self.root)

    def test_repository_limits_match_product_code(self) -> None:
        """rule: INV.RUNTIME.RESOURCE-LIMIT-EVIDENCE"""
        self.assertEqual(limits.check(ROOT), [])

    def test_accepts_described_limits_and_ignores_test_constants(self) -> None:
        self.assertEqual(self.errors(), [])
        found, _ = limits.inventory(self.root)
        self.assertEqual([entry["id"] for entry in found["limits"]], ["demo.file-bytes", "demo.read-timeout", "demo.literal-bytes"])
        self.assertEqual(found["limits"][0]["expr"], "8 * 1024 * 1024")
        self.assertEqual(found["limits"][0]["at"], self.line_of("pub const MAX_FILE_BYTES"))

    def test_rejects_a_value_that_drifts_from_the_code(self) -> None:
        """rule: INV.RUNTIME.RESOURCE-LIMIT-EVIDENCE"""
        self.lib(block(value="1"))
        self.assertTrue(any("evaluates to 8388608, the block says 1" in error for error in self.errors()), self.errors())

    def test_rejects_a_limit_constant_without_a_block(self) -> None:
        """rule: INV.RUNTIME.RESOURCE-LIMIT-EVIDENCE"""
        self.lib(block(), timeout="")
        errors = self.errors()
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("READ_TIMEOUT has no `// resource-limit:` block above it", errors[0])

    def test_rejects_a_block_that_stands_before_no_limit(self) -> None:
        """rule: INV.RUNTIME.RESOURCE-LIMIT-EVIDENCE"""
        self.lib(block() + "\n")
        errors = self.errors()
        self.assertTrue(any("demo.file-bytes stands before no constant" in error for error in errors), errors)
        self.assertTrue(any("MAX_FILE_BYTES has no" in error for error in errors), errors)

    def test_rejects_a_repeated_id(self) -> None:
        self.lib(block("demo.read-timeout"))
        self.assertTrue(any("id demo.read-timeout is already used" in error for error in self.errors()), self.errors())

    def test_rejects_a_moved_literal(self) -> None:
        self.lib(block(), literal=LITERAL.replace("// count: 1", "// count: 2"))
        self.assertTrue(any("occurs 1 times" in error for error in self.errors()), self.errors())
        self.lib(block(), literal=LITERAL + "\n")
        self.assertTrue(any("stands before no line with literal" in error for error in self.errors()), self.errors())

    def test_rejects_a_literal_whose_expression_is_not_in_it(self) -> None:
        self.lib(block(), literal=LITERAL.replace("// expr: 32 * 1024 * 1024", "// expr: 16 * 1024 * 1024"))
        self.assertTrue(any("is not part of literal" in error for error in self.errors()), self.errors())

    def test_exclusion_covers_a_constant_that_is_not_a_limit(self) -> None:
        self.lib(block(), timeout="// resource-limit-excluded: демонстрация\n")
        self.assertEqual(self.errors(), [])
        found, _ = limits.inventory(self.root)
        self.assertEqual(found["excluded"], [{"at": self.line_of("const READ_TIMEOUT"), "symbol": "READ_TIMEOUT", "reason": "демонстрация"}])

    def test_rejects_an_exclusion_that_stands_before_no_constant(self) -> None:
        """rule: INV.RUNTIME.RESOURCE-LIMIT-EVIDENCE"""
        self.lib(block(), timeout="// resource-limit-excluded: демонстрация\n\n")
        errors = self.errors()
        self.assertTrue(any("resource-limit-excluded stands before no constant" in error for error in errors), errors)
        self.assertTrue(any("READ_TIMEOUT has no" in error for error in errors), errors)
        found, _ = limits.inventory(self.root)
        self.assertEqual(found["excluded"], [])

    def test_justified_status_requires_evidence_and_decision(self) -> None:
        """rule: INV.RUNTIME.RESOURCE-LIMIT-EVIDENCE"""
        self.lib(block(status="justified"))
        self.assertTrue(any("a justified limit names its evidence and decision" in error for error in self.errors()))
        self.lib(block(status="external"))
        self.assertTrue(any("an external limit names its origin" in error for error in self.errors()))
        self.lib(
            block(
                status="justified",
                evidence=["замер на УТ"],
                decision="предел закреплён",
                decision_source="#1119",
            )
        )
        self.assertEqual(self.errors(), [])

    def test_a_decision_names_its_source_and_no_date(self) -> None:
        """rule: INV.RUNTIME.RESOURCE-LIMIT-EVIDENCE"""
        justified = {"status": "justified", "evidence": ["замер на УТ"], "decision": "предел закреплён"}
        self.lib(block(**justified))
        self.assertTrue(any("decision and decision-source go together" in error for error in self.errors()), self.errors())
        dated = block(**justified, decision_source="#1119").replace(
            "// decision-source: #1119\n", "// decision-date: 2026-10-25\n// decision-source: #1119\n"
        )
        self.lib(dated)
        self.assertTrue(any("unknown field `decision-date`" in error for error in self.errors()), self.errors())
        self.lib(block(**justified, decision_source="arch/rules/runtime/resource-limit-evidence.md"))
        found, errors = limits.inventory(self.root)
        self.assertEqual(errors, [])
        self.assertEqual(
            found["limits"][0]["decision"],
            {"text": "предел закреплён", "source": "arch/rules/runtime/resource-limit-evidence.md"},
        )

    def test_rejects_an_unknown_class_field_or_order(self) -> None:
        self.lib(block(**{"class": "misc"}))
        self.assertTrue(any("class `misc` is not one of" in error for error in self.errors()), self.errors())
        self.lib(block().replace("// status: unproven\n", "// state: unproven\n"))
        self.assertTrue(any("unknown field `state`" in error for error in self.errors()), self.errors())
        self.lib(block().replace("// unit: bytes\n// scope: file\n", "// scope: file\n// unit: bytes\n"))
        self.assertTrue(any("fields go in the order" in error for error in self.errors()), self.errors())

    def test_an_unknown_origin_or_problem_is_said_and_not_left_out(self) -> None:
        """rule: INV.RUNTIME.RESOURCE-LIMIT-EVIDENCE"""
        self.lib(block(origin=None))
        self.assertTrue(any("field `origin` is missing" in error for error in self.errors()), self.errors())
        self.lib(block(problem="растёт память демона"))
        found, errors = limits.inventory(self.root)
        self.assertEqual(errors, [])
        self.assertEqual((found["limits"][0]["origin"], found["limits"][0]["problem"]), (None, "растёт память демона"))

    def test_lines_end_only_at_a_line_feed_as_the_parser_counts_them(self) -> None:
        self.write("crates/demo/src/read.rs", "use crate::MAX_FILE_BYTES;\nconst NOTE: &str = \"a b\";\n")
        self.write(
            "crates/demo/src/read.rs",
            "const NOTE: &str = \"a b\";\n" + block("demo.read-bytes", value="3") + "const MAX_READ_BYTES: usize = 3;\n",
        )
        self.assertEqual(self.errors(), [])

    def test_scanner_reads_past_an_early_test_module_declaration(self) -> None:
        """rule: INV.RUNTIME.RESOURCE-LIMIT-EVIDENCE"""
        self.write(
            "crates/demo/src/lib.rs",
            f"#[cfg(test)]\nmod helpers;\nmod read;\n\n{block()}pub const MAX_FILE_BYTES: usize =\n    8 * 1024 * 1024;\n"
            f"{TIMEOUT}const READ_TIMEOUT: Duration = Duration::from_secs(30);\n"
            "struct Store;\nimpl Store {\n    const MAX_ROWS: usize = 64;\n}\n",
        )
        self.write("crates/demo/src/helpers.rs", "pub const MAX_HELPER_BYTES: usize = 1;\n")
        errors = self.errors()
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("MAX_ROWS has no", errors[0])

    def test_platform_code_that_tests_also_build_stays_in_the_product(self) -> None:
        self.write(
            "crates/demo/src/read.rs",
            "use crate::MAX_FILE_BYTES;\n#[cfg(any(windows, test))]\nconst MAX_WINDOWS_BYTES: usize = 3;\n"
            "#[cfg(any(test, feature = \"test-support\"))]\nconst MAX_SUPPORT_BYTES: usize = 4;\n"
            "#[cfg(all(unix, not(test)))]\nconst MAX_UNIX_BYTES: usize = 5;\n",
        )
        errors = sorted(self.errors())
        self.assertEqual(len(errors), 2, errors)
        self.assertIn("MAX_WINDOWS_BYTES has no", errors[0])
        self.assertIn("MAX_UNIX_BYTES has no", errors[1])

    def test_same_named_limits_are_told_apart_by_their_place(self) -> None:
        view = block("demo.view-page", **{"class": "step", "unit": "count", "scope": "request", "value": "20"})
        diff = block("demo.diff-page", **{"class": "step", "unit": "count", "scope": "request", "value": "100"})
        self.write("crates/demo/src/view.rs", f"{view}const DEFAULT_LIMIT: usize = 20;\n")
        self.write("crates/demo/src/diff.rs", f"{diff}const DEFAULT_LIMIT: usize = 100;\n")
        self.assertEqual(self.errors(), [])

    def test_derived_value_is_checked_against_its_neighbours(self) -> None:
        total = block("demo.total-bytes", value="1", derived="true")
        self.write("crates/demo/src/total.rs", f"{total}const MAX_TOTAL_BYTES: usize = MAX_FILE_BYTES * 2;\n")
        self.assertTrue(any("evaluates to 16777216" in error for error in self.errors()), self.errors())
        self.write("crates/demo/src/total.rs", f"{block('demo.total-bytes', value='16777216')}const MAX_TOTAL_BYTES: usize = MAX_FILE_BYTES * 2;\n")
        self.assertTrue(any("mark it `derived: true`" in error for error in self.errors()), self.errors())

    def test_evaluates_byte_and_duration_expressions(self) -> None:
        self.assertEqual(limits.evaluate("64 * 1024 * 1024", "bytes"), 64 * 1024 * 1024)
        self.assertEqual(limits.evaluate("65_536", "count"), 65_536)
        self.assertEqual(limits.evaluate("1u64 << 20", "bytes"), 1 << 20)
        self.assertEqual(limits.evaluate("Duration::from_millis(500)", "ms"), 500)
        self.assertEqual(limits.evaluate("Duration::from_secs(2)", "ms"), 2000)
        self.assertIsNone(limits.evaluate("Duration::from_secs(2)", "count"))
        self.assertIsNone(limits.evaluate("OTHER_LIMIT", "count"))


if __name__ == "__main__":
    unittest.main()
