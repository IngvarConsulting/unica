"""Каждая ссылка `check` в `arch/rules/` указывает на существующий тест.

Перенос теста в другой файл или крейт без правки правила оставляет обязательство
без доказательства: так при разделении на крейты (#1861) семь ссылок на тесты
ролей остались на старом месте. Страж разрешает адрес по исходнику, не запуская
сборку: Rust — `путь.rs::[модуль::]имя` (функция с атрибутом теста или первый
аргумент макроса этого файла, порождающего тест), Python — `путь.py::Класс.метод`
с методом `test*`.
"""

from __future__ import annotations

import ast
import re
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
RULES = Path("arch/rules")
FRONT_MATTER = re.compile(r"\A---\n(.*?)\n---", re.S)
RUST_ADDRESS = re.compile(r"^(crates/\S+\.rs)::((?:\w+::)*)(\w+)$")
PYTHON_ADDRESS = re.compile(r"^(\S+\.py)::(\w+)\.(\w+)$")


def check_addresses(text: str) -> list[str]:
    match = FRONT_MATTER.match(text)
    if match is None:
        return []
    addresses: list[str] = []
    in_check = False
    for line in match.group(1).splitlines():
        if re.match(r"^\S", line):
            in_check = line.startswith("check:")
        elif in_check and line.strip().startswith("- "):
            addresses.append(line.strip()[2:].strip())
    return addresses


def without_line_comments(source: str) -> str:
    return re.sub(r"//[^\n]*", "", source)


def has_test_attribute(lines: list[str], index: int) -> bool:
    """Атрибуты над строкой `fn` должны объявлять тест."""
    cursor = index - 1
    while cursor >= 0 and (lines[cursor].strip().startswith("#[") or lines[cursor].strip().endswith("]")):
        if re.search(r"#\[\s*(?:\w+::)*test\b", lines[cursor]):
            return True
        cursor -= 1
    return False


def rust_defines(source: str, modules: list[str], name: str) -> bool:
    source = without_line_comments(source)
    for module in modules:
        if not re.search(rf"\bmod\s+{module}\b", source):
            return False
    lines = source.splitlines()
    for index, line in enumerate(lines):
        if re.search(rf"\bfn\s+{name}\b", line) and has_test_attribute(lines, index):
            return True
    # Тест порождает макрос этого файла, получая имя первым аргументом.
    for invocation in re.finditer(rf"\b(\w+)!\s*[(\[{{]\s*(?:#\[[^\]]*\]\s*)*{name}\b", source):
        definition = re.search(rf"macro_rules!\s*{invocation.group(1)}\s*\{{", source)
        if definition and re.search(r"#\[\s*(?:\w+::)*test\b", source[definition.end():definition.end() + 4000]):
            return True
    return False


def python_defines(source: str, cls: str, method: str) -> bool:
    if not method.startswith("test"):
        return False
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.ClassDef) and node.name == cls:
            return any(
                isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name == method
                for item in node.body
            )
    return False


def unresolved(root: Path) -> list[str]:
    problems: list[str] = []
    for rule in sorted((root / RULES).glob("**/*.md")):
        for address in check_addresses(rule.read_text(encoding="utf-8")):
            where = f"{rule.relative_to(root)}: {address}"
            if rust := RUST_ADDRESS.match(address):
                path, modules, name = rust.groups()
                source = root / path
                if not source.is_file():
                    problems.append(f"{where}: no such file")
                elif not rust_defines(
                    source.read_text(encoding="utf-8"), [m for m in modules.split("::") if m], name
                ):
                    problems.append(f"{where}: test is not defined in this file")
            elif python := PYTHON_ADDRESS.match(address):
                path, cls, method = python.groups()
                source = root / path
                if not source.is_file():
                    problems.append(f"{where}: no such file")
                elif not python_defines(source.read_text(encoding="utf-8"), cls, method):
                    problems.append(f"{where}: test is not defined in this file")
            else:
                problems.append(f"{where}: unrecognized address form")
    return problems


class RuleCheckReferenceTests(unittest.TestCase):
    def test_every_rule_check_names_an_existing_test(self) -> None:
        self.assertEqual(unresolved(REPO_ROOT), [])

    def test_rejects_moved_missing_and_malformed_references(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / RULES).mkdir(parents=True)
            (root / "crates/a/src").mkdir(parents=True)
            (root / "tests/ci").mkdir(parents=True)
            (root / "crates/a/src/lib.rs").write_text(
                "macro_rules! case {\n    ($name:ident, $value:expr) => {\n"
                "        #[test]\n        fn $name() { assert!($value > 0); }\n    };\n}\n"
                "mod inner {\n    #[test]\n    #[ignore]\n    fn present() {}\n}\n"
                "case!(\n    generated_case,\n    1\n);\n"
                "fn helper() {}\n"
                "// fn commented_out() {}\n"
                "#[test]\nfn uses_flag() { let flag = true; assert!(flag); }\n",
                encoding="utf-8",
            )
            (root / "tests/ci/test_x.py").write_text(
                "class Suite:\n    def test_ok(self):\n        pass\n\n    def helper(self):\n        pass\n\n"
                "def test_free():\n    pass\n",
                encoding="utf-8",
            )
            (root / RULES / "rule.md").write_text(
                "---\nid: X\ncheck:\n"
                "  - crates/a/src/lib.rs::present\n"
                "  - crates/a/src/lib.rs::inner::present\n"
                "  - crates/a/src/lib.rs::generated_case\n"
                "  - tests/ci/test_x.py::Suite.test_ok\n"
                "  - crates/a/src/lib.rs::moved_away\n"
                "  - crates/a/src/lib.rs::absent_module::present\n"
                "  - crates/b/src/lib.rs::present\n"
                "  - crates/a/src/lib.rs::helper\n"
                "  - crates/a/src/lib.rs::commented_out\n"
                "  - crates/a/src/lib.rs::flag\n"
                "  - tests/ci/test_x.py::Suite.test_free\n"
                "  - tests/ci/test_x.py::Suite.helper\n"
                "  - tests/ci/test_x.py::test_free\n"
                "gap: []\n---\n\nText mentioning crates/a/src/lib.rs::ignored_outside_front_matter\n",
                encoding="utf-8",
            )
            problems = unresolved(root)
        self.assertEqual(
            [problem.split(": ", 1)[1] for problem in problems],
            [
                "crates/a/src/lib.rs::moved_away: test is not defined in this file",
                "crates/a/src/lib.rs::absent_module::present: test is not defined in this file",
                "crates/b/src/lib.rs::present: no such file",
                "crates/a/src/lib.rs::helper: test is not defined in this file",
                "crates/a/src/lib.rs::commented_out: test is not defined in this file",
                "crates/a/src/lib.rs::flag: test is not defined in this file",
                "tests/ci/test_x.py::Suite.test_free: test is not defined in this file",
                "tests/ci/test_x.py::Suite.helper: test is not defined in this file",
                "tests/ci/test_x.py::test_free: unrecognized address form",
            ],
        )


if __name__ == "__main__":
    unittest.main()
