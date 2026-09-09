"""Страж размера: выражение `medium` покрывает каждый файл с процессом или сокетом.

Страж не знает наших имён: признак — конструкции стандартной библиотеки и
Cargo. Выражение объявлено в `.config/nextest.toml` дважды — воротами `pr`
и сроком `medium` — и обе копии обязаны совпадать.

Покрытие проверяется на двух глубинах. Файл обязан быть назван в выражении, а
внутри своего терма обязан перечислить все встроенные модули тестов: новый
`mod *_tests` в уже помеченном файле меняет вывод генератора, и без второй
проверки его тесты молча остаются в воротах `pr`.

Вторая проверка читает исходники, а не `cargo`, поэтому тестов, порождённых
макросом, она не видит — это та же цена, что страж уже платит за отказ от
`cargo`; недосмотр безопасен, страж лишь недосчитается модуля. Обратная
сторона опаснее: модуль тестов за `cfg(feature = …)` страж потребует назвать,
а генератор с фичами по умолчанию его не выведет. Сегодня таких модулей в
крейтах нет; если появятся — сверять придётся по одному набору фич.
"""

from __future__ import annotations

import importlib.util
import re
import tomllib
import unittest
from pathlib import Path

from tree_sitter import Language, Parser
import tree_sitter_rust


REPO_ROOT = Path(__file__).resolve().parents[2]
NEXTEST_TOML = REPO_ROOT / ".config" / "nextest.toml"
# `#[test]` и `#[tokio::test]` дают тест-кейс; `#[cfg(test)]` — нет: у него скобки.
TEST_ATTRIBUTE = re.compile(r"#\[(?:[A-Za-z0-9_]+::)*test\]")
# Терм перечисления: `test(/^модуль::(a|b)::/)`. Второй вид — тесты на верхнем
# уровне файла: `test(/^модуль::[^:]+$/)`.
ENUMERATED_TERM = re.compile(r"test\(/\^([A-Za-z0-9_:]+)::\(([^)]*)\)::/\)")
DIRECT_TERM = re.compile(r"test\(/\^([A-Za-z0-9_:]+)::\[\^:\]\+\$/\)")


def load_size_filters():
    spec = importlib.util.spec_from_file_location("size_filters", REPO_ROOT / "scripts" / "ci" / "size-filters.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def inline_test_modules(source: bytes) -> tuple[set[str], bool]:
    """Встроенные модули файла с тестами внутри и признак тестов на его верхнем уровне.

    Генератор относит тест к первому сегменту после модуля файла, поэтому
    вложенный модуль засчитывается внешнему, а не называется отдельно.
    """
    tree = Parser(Language(tree_sitter_rust.language())).parse(source)

    def is_test_attribute(node) -> bool:
        if node.type != "attribute_item":
            return False
        compact = re.sub(r"\s+", "", source[node.start_byte : node.end_byte].decode())
        return TEST_ATTRIBUTE.fullmatch(compact) is not None

    def holds_test(node) -> bool:
        stack = [node]
        while stack:
            current = stack.pop()
            if is_test_attribute(current):
                return True
            stack.extend(current.children)
        return False

    modules: set[str] = set()
    top_level = False
    children = tree.root_node.named_children
    for index, child in enumerate(children):
        # Модуль без тела — отдельный файл; его тесты принадлежат не этому терму.
        if child.type == "mod_item" and child.child_by_field_name("body") is not None:
            if holds_test(child):
                modules.add(child.child_by_field_name("name").text.decode())
        elif child.type == "function_item":
            # Атрибут — сиблинг перед элементом, а не его ребёнок.
            back = index - 1
            while back >= 0 and children[back].type == "attribute_item":
                top_level = top_level or is_test_attribute(children[back])
                back -= 1
    return modules, top_level


class InlineTestModuleReadingTests(unittest.TestCase):
    """Разбор исходника закреплён: молча ослепший разбор снова обнулил бы стража."""

    def test_reads_what_the_generator_would_name(self) -> None:
        cases = {
            "встроенный модуль с тестами": (b"mod a_tests { #[test] fn t() {} }", ({"a_tests"}, False)),
            "тесты tokio внутри модуля": (b"mod b_tests { #[tokio::test] async fn t() {} }", ({"b_tests"}, False)),
            "cfg(test) сам по себе не тест": (b"#[cfg(test)] mod c { fn helper() {} }", (set(), False)),
            "модуль-файл без тела": (b"mod d;", (set(), False)),
            "модуль без тестов": (b"mod e { fn helper() {} }", (set(), False)),
            "тест на верхнем уровне": (b"#[test]\nfn t() {}", (set(), True)),
            "тест tokio на верхнем уровне": (b"#[tokio::test]\nasync fn t() {}", (set(), True)),
            "тест за чужим атрибутом": (b'#[cfg(feature="x")]\n#[test]\nfn t() {}', (set(), True)),
            "вложенный модуль засчитан внешнему": (b"mod outer { mod inner { #[test] fn t() {} } }", ({"outer"}, False)),
            "функция без атрибута": (b"fn plain() {}", (set(), False)),
        }

        for name, (source, expected) in cases.items():
            with self.subTest(case=name):
                self.assertEqual(inline_test_modules(source), expected)


class SizeGuardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.config = tomllib.loads(NEXTEST_TOML.read_text(encoding="utf-8"))
        self.deadline = next(o for o in self.config["profile"]["default"]["overrides"] if "slow-timeout" in o)
        self.medium = self.deadline["filter"].strip()

    def test_pr_gate_and_medium_deadline_share_one_expression(self) -> None:
        pr = self.config["profile"]["pr"]["default-filter"].strip()

        self.assertEqual(pr, f"not (\n{self.medium}\n)")
        self.assertEqual(self.deadline["slow-timeout"], {"period": "300s", "terminate-after": 2})
        self.assertTrue(self.medium.startswith("kind(test)"))

    def test_every_module_with_a_process_or_socket_is_declared_medium(self) -> None:
        """Файл с `std::process` или `std::net` не бывает `small` молча."""
        module = load_size_filters()
        declared = set(re.findall(r"test\(/\^([A-Za-z0-9_:]+)::", self.medium))

        for crate, path, _ in module.flagged_modules(REPO_ROOT):
            with self.subTest(module=f"{crate}::{path}"):
                self.assertIn(path, declared)

    def test_every_inline_test_module_is_named_in_its_term(self) -> None:
        """Новый `mod *_tests` в помеченном файле не уезжает в ворота `pr` молча."""
        module = load_size_filters()
        enumerated: dict[str, set[str]] = {}
        for match in ENUMERATED_TERM.finditer(self.medium):
            enumerated.setdefault(match.group(1), set()).update(match.group(2).split("|"))
        direct = set(DIRECT_TERM.findall(self.medium))
        remedy = "перезапустите `python3 scripts/ci/size-filters.py --write`"

        for crate, path, source in module.flagged_modules(REPO_ROOT):
            modules, top_level = inline_test_modules(source.read_bytes())
            with self.subTest(module=f"{crate}::{path}"):
                self.assertEqual(
                    sorted(modules - enumerated.get(path, set())),
                    [],
                    f"встроенные модули тестов не названы в терме — {remedy}",
                )
                if top_level:
                    self.assertIn(path, direct, f"тесты верхнего уровня не названы термом — {remedy}")


if __name__ == "__main__":
    unittest.main()
