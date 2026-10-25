from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("resource_limits", ROOT / "scripts/ci/check-resource-limits.py")
limits = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(limits)


def entry(**overrides) -> dict:
    value = {
        "id": "demo.file-bytes",
        "class": "corpus",
        "resource": "байты одного файла",
        "unit": "bytes",
        "scope": "file",
        "source": {"file": "crates/demo/src/lib.rs", "symbol": "MAX_FILE_BYTES"},
        "expr": "8 * 1024 * 1024",
        "value": 8 * 1024 * 1024,
        "applied": ["crates/demo/src/lib.rs", "crates/demo/src/read.rs"],
        "status": "unproven",
        "origin": None,
        "problem": None,
        "onReach": "отказ source_unreadable",
        "issue": "#1119",
        "evidence": [],
        "decision": None,
        "review": None,
    }
    value.update(overrides)
    return value


class ResourceLimitRegistryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "arch").mkdir()
        shutil.copy(ROOT / limits.SCHEMA, self.root / limits.SCHEMA)
        self.write(
            "crates/demo/src/lib.rs",
            "pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;\n"
            "const READ_TIMEOUT: Duration = Duration::from_secs(30);\n"
            "fn read() { let _ = vec![0u8; 32 * 1024 * 1024]; }\n"
            "#[cfg(test)]\nmod tests {\n    const MAX_TEST_BYTES: usize = 1;\n}\n",
        )
        self.write("crates/demo/src/read.rs", "use crate::MAX_FILE_BYTES;\n")
        self.write("crates/demo/tests/large.rs", "const MAX_FIXTURE_BYTES: usize = 9;\nuse demo::MAX_FILE_BYTES;\n")
        self.timeout = entry(
            id="demo.read-timeout",
            **{"class": "deadline", "unit": "ms", "scope": "request", "issue": "#1312", "onReach": "отказ deadline_exceeded"},
            source={"file": "crates/demo/src/lib.rs", "symbol": "READ_TIMEOUT"},
            expr="Duration::from_secs(30)",
            value=30_000,
            applied=["crates/demo/src/lib.rs"],
        )
        self.literal = entry(
            id="demo.literal-bytes",
            source={"file": "crates/demo/src/lib.rs", "literal": "32 * 1024 * 1024", "count": 1},
            expr="32 * 1024 * 1024",
            value=32 * 1024 * 1024,
            applied=["crates/demo/src/lib.rs"],
        )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write(self, relative: str, text: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def registry(self, items: list[dict], excluded: list[dict] | None = None) -> None:
        payload = {"schema": 1, "limits": items, "excluded": excluded or []}
        (self.root / limits.REGISTRY).write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")

    def test_repository_registry_matches_product_code(self) -> None:
        self.assertEqual(limits.check(ROOT), [])

    def test_accepts_a_complete_registry_and_ignores_test_constants(self) -> None:
        self.registry([entry(), self.timeout, self.literal])
        self.assertEqual(limits.check(self.root), [])

    def test_rejects_a_value_that_drifts_from_the_code(self) -> None:
        self.registry([entry(expr="16 * 1024 * 1024", value=16 * 1024 * 1024), self.timeout, self.literal])
        self.assertTrue(any("code has `8 * 1024 * 1024`" in error for error in limits.check(self.root)))
        self.registry([entry(value=1), self.timeout, self.literal])
        self.assertTrue(any("evaluates to 8388608" in error for error in limits.check(self.root)))

    def test_rejects_an_unregistered_limit_constant(self) -> None:
        self.registry([entry(), self.literal])
        errors = limits.check(self.root)
        self.assertEqual(len(errors), 1)
        self.assertIn("READ_TIMEOUT is not in arch/resource-limits.json", errors[0])

    def test_rejects_a_stale_entry_and_a_stale_exclusion(self) -> None:
        self.registry(
            [entry(), self.timeout, self.literal, entry(id="demo.gone", source={"file": "crates/demo/src/lib.rs", "symbol": "MAX_GONE"})],
            [{"file": "crates/demo/src/lib.rs", "symbol": "MAX_OLD", "reason": "не предел"}],
        )
        errors = limits.check(self.root)
        self.assertTrue(any("no product limit constant MAX_GONE" in error for error in errors))
        self.assertTrue(any("excluded crates/demo/src/lib.rs::MAX_OLD no longer exists" in error for error in errors))

    def test_rejects_incomplete_application_sites(self) -> None:
        self.registry([entry(applied=["crates/demo/src/lib.rs"]), self.timeout, self.literal])
        self.assertTrue(any("applied files" in error for error in limits.check(self.root)))

    def test_rejects_a_moved_literal(self) -> None:
        self.registry([entry(), self.timeout, {**self.literal, "source": {**self.literal["source"], "count": 2}}])
        self.assertTrue(any("occurs 1 times" in error for error in limits.check(self.root)))

    def test_exclusion_covers_a_constant_that_is_not_a_limit(self) -> None:
        self.registry(
            [entry(), self.literal],
            [{"file": "crates/demo/src/lib.rs", "symbol": "READ_TIMEOUT", "reason": "демонстрация"}],
        )
        self.assertEqual(limits.check(self.root), [])

    def test_justified_status_requires_evidence_and_decision(self) -> None:
        self.registry([entry(status="justified"), self.timeout, self.literal])
        errors = limits.check(self.root)
        self.assertTrue(any("limits/0/evidence" in error for error in errors), errors)
        self.assertTrue(any("limits/0/decision" in error for error in errors), errors)
        self.registry([entry(status="external"), self.timeout, self.literal])
        self.assertTrue(any("limits/0/origin" in error for error in limits.check(self.root)))

    def test_schema_rejects_an_unknown_class(self) -> None:
        self.registry([entry(**{"class": "misc"}), self.timeout, self.literal])
        self.assertTrue(any("class" in error for error in limits.check(self.root)))

    def test_scanner_reads_past_an_early_test_module_declaration(self) -> None:
        self.write(
            "crates/demo/src/lib.rs",
            "#[cfg(test)]\nmod helpers;\nmod read;\n\npub const MAX_FILE_BYTES: usize =\n    8 * 1024 * 1024;\n"
            "const READ_TIMEOUT: Duration = Duration::from_secs(30);\n"
            "fn read() { let _ = vec![0u8; 32 * 1024 * 1024]; }\n"
            "struct Store;\nimpl Store {\n    const MAX_ROWS: usize = 64;\n}\n",
        )
        self.write("crates/demo/src/helpers.rs", "pub const MAX_HELPER_BYTES: usize = 1;\n")
        self.registry([entry(), self.timeout, self.literal])
        errors = limits.check(self.root)
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("MAX_ROWS is not in", errors[0])
        rows = entry(
            id="demo.store-rows",
            **{"class": "format", "unit": "count", "scope": "record", "issue": None},
            source={"file": "crates/demo/src/lib.rs", "symbol": "MAX_ROWS", "within": "impl Store"},
            expr="64",
            value=64,
            applied=["crates/demo/src/lib.rs"],
        )
        self.registry([entry(), self.timeout, self.literal, rows])
        self.assertEqual(limits.check(self.root), [])

    def test_platform_code_that_tests_also_build_stays_in_the_product(self) -> None:
        self.write(
            "crates/demo/src/read.rs",
            "use crate::MAX_FILE_BYTES;\n#[cfg(any(windows, test))]\nconst MAX_WINDOWS_BYTES: usize = 3;\n"
            "#[cfg(any(test, feature = \"test-support\"))]\nconst MAX_SUPPORT_BYTES: usize = 4;\n"
            "#[cfg(all(unix, not(test)))]\nconst MAX_UNIX_BYTES: usize = 5;\n",
        )
        self.registry([entry(), self.timeout, self.literal])
        errors = sorted(limits.check(self.root))
        self.assertEqual(len(errors), 2, errors)
        self.assertIn("MAX_WINDOWS_BYTES is not in", errors[0])
        self.assertIn("MAX_UNIX_BYTES is not in", errors[1])

    def test_same_named_limits_in_one_crate_keep_their_own_users(self) -> None:
        self.write("crates/demo/src/view.rs", "const DEFAULT_LIMIT: usize = 20;\nfn page() -> usize { DEFAULT_LIMIT }\n")
        self.write("crates/demo/src/diff.rs", "const DEFAULT_LIMIT: usize = 100;\nfn page() -> usize { DEFAULT_LIMIT }\n")
        view = entry(
            id="demo.view-page",
            **{"class": "step", "unit": "count", "scope": "request", "issue": None},
            source={"file": "crates/demo/src/view.rs", "symbol": "DEFAULT_LIMIT"},
            expr="20",
            value=20,
            applied=["crates/demo/src/diff.rs", "crates/demo/src/view.rs"],
        )
        diff = {**view, "id": "demo.diff-page", "source": {"file": "crates/demo/src/diff.rs", "symbol": "DEFAULT_LIMIT"},
                "expr": "100", "value": 100, "applied": ["crates/demo/src/diff.rs"]}
        self.registry([entry(), self.timeout, self.literal, view, diff])
        errors = limits.check(self.root)
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("demo.view-page: applied files", errors[0])

    def test_rejects_a_literal_whose_expression_is_not_in_it(self) -> None:
        self.registry([entry(), self.timeout, {**self.literal, "expr": "16 * 1024 * 1024", "value": 16 * 1024 * 1024}])
        self.assertTrue(any("is not part of literal" in error for error in limits.check(self.root)))

    def test_derived_value_is_checked_against_its_neighbours(self) -> None:
        self.write(
            "crates/demo/src/lib.rs",
            "pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;\nconst MAX_TOTAL_BYTES: usize = MAX_FILE_BYTES * 2;\n"
            "const READ_TIMEOUT: Duration = Duration::from_secs(30);\nfn read() { let _ = vec![0u8; 32 * 1024 * 1024]; }\n",
        )
        total = entry(
            id="demo.total-bytes",
            source={"file": "crates/demo/src/lib.rs", "symbol": "MAX_TOTAL_BYTES"},
            expr="MAX_FILE_BYTES * 2",
            value=1,
            derived=True,
            applied=["crates/demo/src/lib.rs"],
        )
        self.registry([entry(), self.timeout, self.literal, total])
        self.assertTrue(any("evaluates to 16777216" in error for error in limits.check(self.root)))

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
