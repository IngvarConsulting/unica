"""Страж новых конечных сроков операций: `scripts/ci/check-finite-deadlines.py`.

Случаи разбора повторяют прежний Rust-страж в `unica-coder`: файл и элемент
под тестовым `cfg` пропускаются, псевдоним из `use` не прячет вызов, условие,
которое собирается и вне тестов, остаётся производственным.
"""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "ci" / "check-finite-deadlines.py"
SPEC = importlib.util.spec_from_file_location("check_finite_deadlines", SCRIPT_PATH)
guard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guard)


class RepositoryTests(unittest.TestCase):
    def test_production_operation_paths_start_no_new_finite_deadline(self) -> None:
        """rule: INV.RUNTIME.TIMERS-PRESERVE-WORK"""
        self.assertEqual(guard.violations(guard.found_calls(REPO_ROOT)), [])

    def test_a_new_call_an_extra_call_and_a_vanished_call_all_fail(self) -> None:
        """rule: INV.RUNTIME.TIMERS-PRESERVE-WORK"""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            listed = root / "crates/unica-lsp/src/host.rs"
            listed.parent.mkdir(parents=True)
            listed.write_text("fn a() { let _ = ProviderDeadline::new(d); }\n", encoding="utf-8")
            found = guard.found_calls(root)
            self.assertEqual(found, {"crates/unica-lsp/src/host.rs": 1})
            missing = {line.split(":")[0] for line in guard.violations(found)}
            self.assertEqual(missing, set(guard.ALLOWED) - {"crates/unica-lsp/src/host.rs"})

            listed.write_text("fn a() { let _ = ProviderDeadline::new(d); let _ = ProviderDeadline::new(e); }\n",
                              encoding="utf-8")
            self.assertIn("crates/unica-lsp/src/host.rs: 2 finite ProviderDeadline constructor call(s), allowed 1",
                          guard.violations(guard.found_calls(root)))

            unlisted = root / "crates/unica-search/src/walk.rs"
            unlisted.parent.mkdir(parents=True)
            unlisted.write_text("fn walk() { let _ = ProviderDeadline::from_budget(B); }\n", encoding="utf-8")
            self.assertIn("crates/unica-search/src/walk.rs: 1 finite ProviderDeadline constructor call(s), allowed 0",
                          guard.violations(guard.found_calls(root)))

    def test_test_sources_and_files_outside_src_are_not_production(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in [
                "crates/a/src/tests.rs", "crates/a/src/x_tests.rs", "crates/a/src/test_support.rs",
                "crates/a/src/test_control.rs", "crates/a/src/testing.rs", "crates/a/src/m/tests/y.rs",
                "crates/a/src/receipt_scenario_v5/z.rs", "crates/a/tests/it.rs", "crates/a/benches/b.rs",
            ]:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fn f() { let _ = ProviderDeadline::new(d); }\n", encoding="utf-8")
            self.assertEqual(guard.found_calls(root), {})


class ParsingTests(unittest.TestCase):
    def test_whole_file_cfg_is_honored_without_hiding_operation_paths(self) -> None:
        for source in [
            '#![cfg(feature = "receipt-ledger-test-support")]\nfn contract() { let _ = ProviderDeadline::new(deadline); }',
            "#![cfg(test)]\nfn fixture() { let _ = ProviderDeadline::from_budget(B); }",
            '#![cfg(all(test, target_os = "macos"))]\nfn fixture() { let _ = ProviderDeadline::from_budget(B); }',
        ]:
            with self.subTest(source=source):
                self.assertEqual(guard.finite_deadline_calls(source), 0)
        for source in [
            "fn contract() { let _ = ProviderDeadline::new(deadline); }",
            '#![cfg(any(test, feature = "receipt-ledger-test-support"))]\nfn contract() { let _ = ProviderDeadline::new(deadline); }',
            "#![cfg(not(test))]\nfn operation() { let _ = ProviderDeadline::new(deadline); }",
            '#![cfg(feature = "other")]\nfn operation() { let _ = ProviderDeadline::new(deadline); }',
            '#![cfg(not(feature = "receipt-ledger-test-support"))]\nfn operation() { let _ = ProviderDeadline::new(deadline); }',
            '#![cfg(feature = "other")]\nuse unica_work_control::provider_deadline::ProviderDeadline as Budget;\n'
            "fn operation() { let _ = Budget::from_budget(B); }",
        ]:
            with self.subTest(source=source):
                self.assertEqual(guard.finite_deadline_calls(source), 1)

    def test_aliases_are_recognized_and_only_test_code_is_skipped(self) -> None:
        cases = [
            ("fn read() { let _ = ProviderDeadline::from_budget(Duration::from_secs(120)); }", 1),
            ("fn read() { let _ = unica_work_control::provider_deadline::ProviderDeadline::new(Instant::now()); }", 1),
            ("use unica_work_control::provider_deadline::ProviderDeadline as Budget;\n"
             "fn read() { let _ = Budget::from_started_at(Instant::now(), BUDGET); }", 1),
            ("use x::{y::ProviderDeadline as Budget, z};\nfn read() { let _ = Budget::new(at); }", 1),
            ("fn read() { let _ = ProviderDeadline::no_deadline(); }", 0),
            ("#[cfg(test)]\nmod tests { fn fixture() { let _ = ProviderDeadline::from_budget(B); } }", 0),
            ('#[cfg(any(test, feature = "hostile"))]\nfn escape() { let _ = ProviderDeadline::from_budget(B); }', 1),
            ("#[cfg(not(test))]\nfn production() { let _ = ProviderDeadline::from_budget(B); }", 1),
            ("#[cfg(all(test, unix))]\nfn fixture() { let _ = ProviderDeadline::new(d); }", 0),
            ('#[cfg(feature = "receipt-ledger-test-support")]\nfn harness() { let _ = ProviderDeadline::new(d); }', 0),
            ("#[cfg(test)]\nuse x::ProviderDeadline as Budget;\nfn read() { let _ = Budget::new(d); }", 0),
            # Unrelated names are not deadlines.
            ("fn read() { let _ = Other::new(d); let _ = ProviderDeadline::remaining(d); }", 0),
        ]
        for source, expected in cases:
            with self.subTest(source=source):
                self.assertEqual(guard.finite_deadline_calls(source), expected)

    def test_test_gates_follow_items_methods_and_statements(self) -> None:
        cases = [
            ("impl X {\n#[cfg(test)]\nfn fixture() { let _ = ProviderDeadline::new(d); }\n"
             "fn operation() { let _ = ProviderDeadline::new(d); } }", 1),
            # A gated constant inside `impl` is still read, as the Rust guard did.
            ("impl X {\n#[cfg(test)]\nconst D: P = ProviderDeadline::new(d); }", 1),
            # A trait's default method is always read.
            ("trait T {\n#[cfg(test)]\nfn d() { let _ = ProviderDeadline::new(d); } }", 1),
            ("fn f() {\n#[cfg(test)]\nlet _ = ProviderDeadline::new(d);\n"
             "#[cfg(test)]\nProviderDeadline::new(d);\n"
             "#[cfg(test)]\nholder.set(ProviderDeadline::from_budget(b));\n"
             "#[cfg(test)]\nif x { ProviderDeadline::new(d); }\n"
             "let _ = ProviderDeadline::new(d); }", 1),
            # A gated block statement is still read, as the Rust guard did.
            ("fn f() {\n#[cfg(test)]\n{ ProviderDeadline::new(d); } }", 1),
            ("fn f() {\n#[cfg(test)]\nfn nested() { let _ = ProviderDeadline::new(d); } }", 0),
            ("#[allow(dead_code)]\n#[cfg(test)]\n// note\nfn f() { let _ = ProviderDeadline::new(d); }", 0),
            ("#[cfg(test)]\nfn f() {}\nfn g() { let _ = ProviderDeadline::new(d); }", 1),
            # Macro arguments are not expanded: the compiler sees the call only after expansion.
            ("fn f() { run!(ProviderDeadline::new(d)); }", 0),
        ]
        for source, expected in cases:
            with self.subTest(source=source):
                self.assertEqual(guard.finite_deadline_calls(source), expected)


if __name__ == "__main__":
    unittest.main()
