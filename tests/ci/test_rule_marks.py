"""Пометки `rule: <ID>` на тестах заменяют ссылки `check` в правилах.

Тест сам называет правило, которое подтверждает; при сборе результатов
пометка становится меткой `rule` отчёта Allure. Пометка с правилом, которого
нет в `arch/rules/`, или пометка не над тестом — ошибка: связь с правилом
должна вести к существующему тесту.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts" / "ci"))
import rule_marks  # noqa: E402

RUST = r'''//! rule: INV.A.FILE
macro_rules! case {
    ($name:ident, $value:expr) => {
        #[test]
        fn $name() { assert!($value > 0); }
    };
}

/// Документация теста.
/// rule: INV.A.ONE
#[test]
fn top() { let text = "{ not a brace }"; let raw = r#"}"#; let c = '{'; }

/// rule: INV.A.MODULE
mod inner {
    /// rule: INV.A.TWO
    #[test]
    #[ignore]
    fn nested() {}

    fn helper<'a>(value: &'a str) -> &'a str { value }
}

// rule: INV.A.MACRO
case!(generated, 1);

/// rule: INV.A.TWO
fn not_a_test() {}
'''

PYTHON = '''class Suite:
    """rule: INV.A.CLASS"""

    def test_doc(self):
        """Проверяет что-то.

        rule: INV.A.ONE
        """

    def helper(self):
        """rule: INV.A.TWO"""
'''


def rule(root: Path, rule_id: str, check: list[str] | None = None) -> None:
    lines = ["---", f"id: {rule_id}"]
    if check is not None:
        lines.append("check:" if check else "check: []")
        lines.extend(f"  - {address}" for address in check)
    lines += ["---", "", "Текст правила.", ""]
    path = root / "arch/rules" / f"{rule_id}.md"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8")


class RuleMarkReadingTests(unittest.TestCase):
    def test_rust_marks_attach_to_tests_modules_macros_and_files(self) -> None:
        pairs = rule_marks.rust_marked_pairs("crates/a/src/lib.rs", RUST)
        self.assertEqual(
            pairs,
            {
                ("INV.A.FILE", "crates/a/src/lib.rs::top"),
                ("INV.A.FILE", "crates/a/src/lib.rs::inner::nested"),
                ("INV.A.FILE", "crates/a/src/lib.rs::generated"),
                ("INV.A.ONE", "crates/a/src/lib.rs::top"),
                ("INV.A.MODULE", "crates/a/src/lib.rs::inner::nested"),
                ("INV.A.TWO", "crates/a/src/lib.rs::inner::nested"),
                ("INV.A.MACRO", "crates/a/src/lib.rs::generated"),
            },
        )

    def test_python_marks_come_from_method_and_class_docstrings(self) -> None:
        self.assertEqual(
            rule_marks.python_marked_pairs("tests/ci/test_x.py", PYTHON),
            {
                ("INV.A.CLASS", "tests/ci/test_x.py::Suite.test_doc"),
                ("INV.A.ONE", "tests/ci/test_x.py::Suite.test_doc"),
            },
        )

    def test_unknown_rules_and_marks_off_a_test_are_reported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for rule_id in ("INV.A.FILE", "INV.A.ONE", "INV.A.MODULE", "INV.A.MACRO", "INV.A.CLASS", "INV.A.TWO"):
                rule(root, rule_id)
            (root / "crates/a/src").mkdir(parents=True)
            (root / "crates/a/src/lib.rs").write_text(RUST.replace("INV.A.MACRO", "INV.A.GONE"), encoding="utf-8")
            (root / "tests/ci").mkdir(parents=True)
            (root / "tests/ci/test_x.py").write_text(PYTHON.replace("INV.A.TWO", "INV.A.HELPER"), encoding="utf-8")
            def line(text: str, needle: str) -> int:
                return text.splitlines().index(needle) + 1

            rust = RUST.replace("INV.A.MACRO", "INV.A.GONE")
            python = PYTHON.replace("INV.A.TWO", "INV.A.HELPER")
            # A used mark of the same rule elsewhere does not hide this one.
            self.assertEqual(
                rule_marks.stray_marks(root),
                [
                    f"crates/a/src/lib.rs:{line(rust, '// rule: INV.A.GONE')}: rule INV.A.GONE is not defined in arch/rules",
                    f"crates/a/src/lib.rs:{line(rust, 'fn not_a_test() {}') - 1}: rule INV.A.TWO is marked, but not on a test",
                    f"tests/ci/test_x.py:{line(python, '        ' + chr(34) * 3 + 'rule: INV.A.HELPER' + chr(34) * 3)}: rule INV.A.HELPER is not defined in arch/rules",
                ],
            )
            (root / "tests/ci/test_x.py").write_text(PYTHON, encoding="utf-8")
            rust = RUST.replace("// rule: INV.A.MACRO\n", "").replace("/// rule: INV.A.TWO\nfn not_a_test", "fn not_a_test")
            (root / "crates/a/src/lib.rs").write_text(rust + "\n/// rule: INV.A.ONE\nmod outside;\n", encoding="utf-8")
            self.assertEqual(
                rule_marks.stray_marks(root),
                [
                    f"crates/a/src/lib.rs:{len(rust.splitlines()) + 2}: rule INV.A.ONE is marked, but not on a test",
                    f"tests/ci/test_x.py:{line(PYTHON, '        ' + chr(34) * 3 + 'rule: INV.A.TWO' + chr(34) * 3)}: rule INV.A.TWO is marked, but not on a test",
                ],
            )


class RuleMarkMigrationTests(unittest.TestCase):
    def test_check_links_become_marks_and_pairs_stay_equal(self) -> None:
        source = (
            "#[cfg(test)]\nmod tests {\n    /// Описание.\n    #[test]\n    fn first() {}\n\n"
            "    #[test]\n    fn second() {}\n}\n"
            "macro_rules! case {\n    ($name:ident) => {\n        #[test]\n        fn $name() {}\n    };\n}\n"
            "case!(generated);\n"
        )
        python = (
            "class Suite:\n    def test_plain(self):\n        pass\n\n"
            '    def test_doc(self):\n        """Однострочный."""\n\n'
            '    def test_long(self):\n        """Первая.\n\n        Вторая.\n        """\n'
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "crates/a/src").mkdir(parents=True)
            (root / "crates/a/src/lib.rs").write_text(source, encoding="utf-8")
            (root / "tests/ci").mkdir(parents=True)
            (root / "tests/ci/test_x.py").write_text(python, encoding="utf-8")
            rule(root, "INV.A.ONE", [
                "crates/a/src/lib.rs::first",
                "crates/a/src/lib.rs::tests::second",
                "crates/a/src/lib.rs::generated",
                "tests/ci/test_x.py::Suite.test_plain",
                "tests/ci/test_x.py::Suite.test_doc",
                "tests/ci/test_x.py::Suite.test_long",
            ])
            rule(root, "INV.A.TWO", ["crates/a/src/lib.rs::tests::first"])
            rule(root, "INV.A.EMPTY", [])
            before, after, problems = rule_marks.migrate(root)
            self.assertEqual(before, after)
            self.assertEqual(len(after), 7)
            self.assertEqual(
                [line for line in problems if not line.startswith("address normalized")], []
            )
            migrated = (root / "crates/a/src/lib.rs").read_text(encoding="utf-8")
            self.assertIn(
                "    /// Описание.\n    /// rule: INV.A.ONE\n    /// rule: INV.A.TWO\n    #[test]\n    fn first", migrated
            )
            self.assertIn("// rule: INV.A.ONE\ncase!(generated);", migrated)
            python_after = (root / "tests/ci/test_x.py").read_text(encoding="utf-8")
            self.assertIn('def test_plain(self):\n        """rule: INV.A.ONE"""\n        pass', python_after)
            self.assertIn('"""Однострочный.\n\n        rule: INV.A.ONE\n        """', python_after)
            self.assertIn("Вторая.\n\n        rule: INV.A.ONE\n        \"\"\"", python_after)
            for path in (root / "arch/rules").glob("*.md"):
                text = path.read_text(encoding="utf-8")
                self.assertNotIn("check", text)
                self.assertIn("Текст правила.", text)
            # Повторный запуск ничего не меняет.
            snapshot = {path: path.read_bytes() for path in root.rglob("*") if path.is_file()}
            again_before, again_after, _ = rule_marks.migrate(root)
            self.assertEqual(again_before, set())
            self.assertEqual(again_after, after)
            self.assertEqual(snapshot, {path: path.read_bytes() for path in root.rglob("*") if path.is_file()})

    def test_unresolved_links_are_reported_and_not_dropped_silently(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "crates/a/src").mkdir(parents=True)
            (root / "crates/a/src/lib.rs").write_text("fn helper() {}\n", encoding="utf-8")
            rule(root, "INV.A.ONE", ["crates/a/src/lib.rs::helper", "crates/b/src/lib.rs::x", "nonsense"])
            _, _, problems = rule_marks.migrate(root)
            self.assertEqual(
                problems,
                [
                    "INV.A.ONE: nonsense: unrecognized address form",
                    "INV.A.ONE: crates/a/src/lib.rs::helper: test is not found or is ambiguous",
                    "INV.A.ONE: crates/b/src/lib.rs::x: no such file",
                ],
            )


def committed(root: Path) -> None:
    """Индекс читает дерево модулей по ревизии `HEAD`, как сборщик на сайте."""

    for command in (
        ["git", "init", "-q"],
        ["git", "add", "-A"],
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", "fixture"],
    ):
        subprocess.run(command, cwd=root, check=True, capture_output=True)


class RuleLabelTests(unittest.TestCase):
    def test_report_entries_receive_the_rules_of_their_tests(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for rule_id in ("INV.A.ONE", "INV.A.TWO", "INV.A.FILE", "INV.A.MODULE", "INV.A.MACRO", "INV.A.CLASS"):
                rule(root, rule_id)
            (root / "crates/a/src/feature").mkdir(parents=True)
            (root / "crates/a/Cargo.toml").write_text('[package]\nname = "a"\n', encoding="utf-8")
            (root / "crates/a/src/lib.rs").write_text("mod feature;\n", encoding="utf-8")
            (root / "crates/a/src/feature/mod.rs").write_text("mod part;\n", encoding="utf-8")
            (root / "crates/a/src/feature/part.rs").write_text(RUST, encoding="utf-8")
            # Unreached files compile into no test and carry no label.
            (root / "crates/a/src/orphan.rs").write_text("/// rule: INV.A.ONE\n#[test]\nfn top() {}\n", encoding="utf-8")
            (root / "crates/a/tests").mkdir(parents=True)
            (root / "crates/a/tests/flow.rs").write_text("/// rule: INV.A.ONE\n#[test]\nfn works() {}\n", encoding="utf-8")
            (root / "tests/ci").mkdir(parents=True)
            (root / "tests/ci/test_x.py").write_text(PYTHON, encoding="utf-8")
            committed(root)
            index = rule_marks.RuleIndex(root)

            def entry(language: str, suite: str, full_name: str) -> dict:
                return {"fullName": full_name, "labels": [{"name": "language", "value": language}, {"name": "suite", "value": suite}]}

            self.assertEqual(
                index.rules_for(entry("rust", "a", "a::feature::part::inner::nested")),
                ["INV.A.FILE", "INV.A.MODULE", "INV.A.TWO"],
            )
            self.assertEqual(index.rules_for(entry("rust", "a::flow", "a::flow::works")), ["INV.A.ONE"])
            self.assertEqual(index.rules_for(entry("rust", "b", "b::feature::part::top")), [])
            self.assertEqual(
                index.rules_for(entry("python", "ci", "test_x.Suite.test_doc")), ["INV.A.CLASS", "INV.A.ONE"]
            )
            self.assertEqual(index.rules_for(entry("python", "ci", "test_x.Suite.helper")), [])


    def test_module_tree_follows_path_attributes_includes_and_declared_test_targets(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rule(root, "INV.A.ONE")
            crate = root / "crates/pkg"
            (crate / "src/impl").mkdir(parents=True)
            (crate / "tests").mkdir(parents=True)
            (crate / "Cargo.toml").write_text(
                '[package]\nname = "pkg"\n\n[[test]]\nname = "other_name"\npath = "tests/flow.rs"\n', encoding="utf-8"
            )
            (crate / "src/lib.rs").write_text(
                '#[path = "impl/server.rs"] mod server;\n'
                'mod inc { include!("included.rs"); }\n',
                encoding="utf-8",
            )
            # A `#[path]` file keeps its child modules next to itself.
            (crate / "src/impl/server.rs").write_text("mod child;\n", encoding="utf-8")
            (crate / "src/impl/child.rs").write_text("/// rule: INV.A.ONE\n#[test]\nfn t() {}\n", encoding="utf-8")
            (crate / "src/included.rs").write_text("/// rule: INV.A.ONE\n#[test]\nfn inc_t() {}\n", encoding="utf-8")
            (crate / "tests/flow.rs").write_text("/// rule: INV.A.ONE\n#[test]\nfn works() {}\n", encoding="utf-8")
            (crate / "tests/second.rs").write_text("#[test]\nfn works() {}\n", encoding="utf-8")
            committed(root)
            index = rule_marks.RuleIndex(root)

            def rules(binary: str, name: str) -> list[str]:
                return index.rules_for(
                    {"fullName": f"{binary}::{name}", "labels": [{"name": "language", "value": "rust"}, {"name": "suite", "value": binary}]}
                )

            self.assertEqual(rules("pkg", "server::child::t"), ["INV.A.ONE"])
            self.assertEqual(rules("pkg", "inc::inc_t"), ["INV.A.ONE"])
            self.assertEqual(rules("pkg::other_name", "works"), ["INV.A.ONE"])
            # Another target of the same package with the same test name stays unlabelled.
            self.assertEqual(rules("pkg::second", "works"), [])
            self.assertEqual(rules("pkg", "works"), [])


class RepositoryRuleMarkTests(unittest.TestCase):
    def test_every_mark_names_a_rule_and_sits_on_a_test(self) -> None:
        self.assertEqual(rule_marks.stray_marks(REPO_ROOT), [])

    def test_rules_no_longer_list_tests(self) -> None:
        self.assertEqual(sorted(rule_marks.check_pairs(REPO_ROOT)), [])


if __name__ == "__main__":
    unittest.main()
