"""Страж размера: выражение `medium` покрывает каждый файл с процессом или сокетом.

Страж не знает наших имён: признак — конструкции стандартной библиотеки и
Cargo. Выражение объявлено в `.config/nextest.toml` дважды — воротами `pr`
и сроком `medium` — и обе копии обязаны совпадать.

Покрытие проверяется на двух глубинах. Файл обязан быть назван в выражении, а
внутри своего терма обязан перечислить все встроенные модули тестов: новый
`mod *_tests` в уже помеченном файле меняет вывод генератора, и без второй
проверки его тесты молча остаются в воротах `pr`.

Что считать тестом, решает разбор ниже — один раз на обе глубины. Второй
копии этого правила нет нигде: `flagged_modules` отдаёт все файлы с процессом
или сокетом, а те, где тестов не нашлось, отсеиваются здесь.

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
import tempfile
import unittest
from copy import deepcopy
from functools import cache
from pathlib import Path

from tree_sitter import Language, Parser
import tree_sitter_rust


REPO_ROOT = Path(__file__).resolve().parents[2]
NEXTEST_TOML = REPO_ROOT / ".config" / "nextest.toml"
# Тест-кейс дают `#[test]`, `#[tokio::test]` и они же со списком аргументов.
# `#[cfg(test)]` — не тест: `test` там аргумент, а не хвост пути атрибута.
TEST_ATTRIBUTE = re.compile(r"#\[(?:[A-Za-z0-9_]+::)*test(?:\(.*\))?\]")
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
    return load_size_filters().inline_test_modules(source)


@cache
def flagged_sources() -> tuple[tuple[str, str, frozenset[str], bool], ...]:
    """Помеченные файлы, в которых нашлись тесты: (крейт, модуль, модули, верхний уровень).

    Файл без тестов пропускается: генератор не выпишет ему терм, объявлять
    нечего. Разбор идёт один раз на обе проверки — их у стража две, а дерево
    одно.
    """
    module = load_size_filters()
    found = []
    for crate, path, source in module.flagged_modules(REPO_ROOT):
        modules, top_level = inline_test_modules(source.read_bytes())
        if modules or top_level:
            found.append((crate, path, frozenset(modules), top_level))
    return tuple(found)


class InlineTestModuleReadingTests(unittest.TestCase):
    """Разбор исходника закреплён: молча ослепший разбор снова обнулил бы стража."""

    def test_reads_what_the_generator_would_name(self) -> None:
        cases = {
            "встроенный модуль с тестами": (b"mod a_tests { #[test] fn t() {} }", ({"a_tests"}, False)),
            "тесты tokio внутри модуля": (b"mod b_tests { #[tokio::test] async fn t() {} }", ({"b_tests"}, False)),
            "тесты tokio с аргументами": (
                b'mod f_tests { #[tokio::test(flavor = "multi_thread")] async fn t() {} }',
                ({"f_tests"}, False),
            ),
            "cfg(test) сам по себе не тест": (b"#[cfg(test)] mod c { fn helper() {} }", (set(), False)),
            "модуль-файл без тела": (b"mod d;", (set(), False)),
            "модуль без тестов": (b"mod e { fn helper() {} }", (set(), False)),
            "тест на верхнем уровне": (b"#[test]\nfn t() {}", (set(), True)),
            "тест tokio на верхнем уровне": (b"#[tokio::test]\nasync fn t() {}", (set(), True)),
            "тест tokio с аргументами на верхнем уровне": (
                b'#[tokio::test(flavor = "multi_thread")]\nasync fn t() {}',
                (set(), True),
            ),
            "тест за чужим атрибутом": (b'#[cfg(feature="x")]\n#[test]\nfn t() {}', (set(), True)),
            "вложенный модуль засчитан внешнему": (b"mod outer { mod inner { #[test] fn t() {} } }", ({"outer"}, False)),
            "функция без атрибута": (b"fn plain() {}", (set(), False)),
        }

        for name, (source, expected) in cases.items():
            with self.subTest(case=name):
                self.assertEqual(inline_test_modules(source), expected)


class ComponentSourceOwnershipTests(unittest.TestCase):
    def test_moved_unit_files_keep_declared_module_paths(self):
        module = load_size_filters()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            src = root / "crates" / "unica-workspace" / "src"
            native = src.parent / "tests" / "platform"
            src.mkdir(parents=True)
            native.mkdir(parents=True)
            (src / "lib.rs").write_text('mod actor; mod store;')
            (src / "actor.rs").write_text('#[path = "actor_local.rs"] mod tests;')
            (src / "actor_local.rs").write_text('use std::process; #[test] fn spawns() {}')
            (src / "store.rs").write_text('#[path = "../tests/platform/fork.rs"] mod fork_tests;')
            (native / "fork.rs").write_text('use std::process; #[test] fn inherits() {}')
            sources = module.source_modules(root)["unica-workspace"]
            self.assertEqual(sources[("actor", "tests")][1], src / "actor_local.rs")
            self.assertEqual(sources[("store", "fork_tests")][1], src / "../tests/platform/fork.rs")
            self.assertNotIn(("actor_local",), sources)
            self.assertEqual({path for _, path, _ in module.flagged_modules(root)}, {
                "actor::tests", "store::fork_tests",
            })

    def test_root_test_modules_are_qualified_by_their_component_binary(self):
        module = load_size_filters()
        module.MEASURED_MEDIUM_CASES = ()
        listed = {"rust-suites": {"unica-network": {
            "kind": "lib", "binary-id": "unica-network", "binary-name": "unica_network",
            "testcases": {"tests::connect": {}},
        }}}
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "lib.rs"
            source.write_text("use std::net; mod tests { #[test] fn connect() {} }")
            terms = module.medium_terms(listed, {"unica-network": {(): (True, source)}})
            self.assertEqual(terms, ["(binary(unica_network) & test(/^(tests)::/))"])
            source.write_text(source.read_text() + " #[cfg(windows)] mod windows_tests { #[test] fn native() {} }")
            terms = module.medium_terms(listed, {"unica-network": {(): (True, source)}})
            self.assertEqual(terms, ["(binary(unica_network) & test(/^(tests|windows_tests)::/))"])


class SizeGuardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.config = tomllib.loads(NEXTEST_TOML.read_text(encoding="utf-8"))
        self.deadline = next(o for o in self.config["profile"]["default"]["overrides"] if "slow-timeout" in o)
        self.medium = self.deadline["filter"].strip()

    def test_pr_gate_excludes_the_shared_medium_and_large_expressions(self) -> None:
        pr = self.config["profile"]["pr"]["default-filter"].strip()

        large = self.config["profile"]["large"]["default-filter"].strip()
        self.assertEqual(pr, f"not (\n{self.medium}\n) & not (\n{large}\n)")
        self.assertEqual(self.deadline["slow-timeout"], {"period": "300s", "terminate-after": 2})
        self.assertTrue(self.medium.startswith("kind(test)"))

    def test_measured_volume_case_is_exact_medium_without_its_layout_neighbor(self) -> None:
        module = load_size_filters()
        binary_id, binary_name, name = module.MEASURED_MEDIUM_CASES[0]
        listed = {"rust-suites": {binary_id: {
            "kind": "lib", "binary-id": binary_id, "binary-name": binary_name,
            "testcases": {name: {}},
        }}}
        terms = module.medium_terms(listed, {})
        self.assertEqual(terms, [f"(binary({binary_name}) & test(/^{re.escape(name)}$/))"])
        self.assertIn(terms[0], self.medium)
        pattern = re.search(r"test\(/(.*?)/\)", terms[0])[1]
        self.assertIsNotNone(re.fullmatch(pattern, name))
        spec = importlib.util.spec_from_file_location("allure_results", REPO_ROOT / "scripts" / "ci" / "allure_results.py")
        results = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(results)
        matcher = results.SizeMatcher.from_toml(NEXTEST_TOML)
        self.assertEqual(matcher.size(binary_id, name), "medium")
        for neighbor in [name + "_neighbor", name.rsplit("::", 1)[0] + "::malformed_project_config_is_a_typed_incomplete_snapshot"]:
            self.assertIsNone(re.fullmatch(pattern, neighbor))
            self.assertNotIn(f"test(/^{re.escape(neighbor)}$/)", self.medium)
            self.assertEqual(matcher.size(binary_id, neighbor), "small")

    def test_measured_volume_case_refuses_missing_renamed_or_wrong_binary(self) -> None:
        module = load_size_filters()
        binary_id, binary_name, name = module.MEASURED_MEDIUM_CASES[0]
        base = {"rust-suites": {binary_id: {
            "kind": "lib", "binary-id": binary_id, "binary-name": binary_name,
            "testcases": {name: {}},
        }}}
        cases = {}
        missing = deepcopy(base)
        missing["rust-suites"][binary_id]["testcases"] = {}
        cases["missing"] = missing
        renamed = deepcopy(base)
        renamed["rust-suites"][binary_id]["testcases"] = {name + "_renamed": {}}
        cases["renamed"] = renamed
        for field, value in [("kind", "bin"), ("binary-id", "another-crate"), ("binary-name", "another_binary")]:
            wrong = deepcopy(base)
            wrong["rust-suites"][binary_id][field] = value
            cases[field] = wrong
        duplicate = deepcopy(base)
        duplicate["rust-suites"]["another-target"] = deepcopy(duplicate["rust-suites"][binary_id])
        cases["duplicate"] = duplicate
        for label, listed in cases.items():
            with self.subTest(case=label), self.assertRaises(ValueError):
                module.medium_terms(listed, {})

    def test_crate_root_belongs_to_lib_and_main_is_not_lost(self) -> None:
        """Корень крейта — за `lib.rs`, и `main.rs` не пропадает из разбора.

        Один ключ на оба корня стоил бы `declared` всех объявлений `mod ...;`,
        живущих в `lib.rs`, а стражу — модулей, объявленных только там.
        """
        module = load_size_filters()

        for crate, modules in module.source_modules(REPO_ROOT).items():
            src = REPO_ROOT / "crates" / crate / "src"
            paths = {path for _, path in modules.values()}
            if (src / "lib.rs").exists():
                with self.subTest(crate=crate, root="lib.rs"):
                    self.assertEqual(modules[()][1], src / "lib.rs")
            if (src / "main.rs").exists():
                with self.subTest(crate=crate, root="main.rs"):
                    self.assertIn(src / "main.rs", paths)

    def medium_declarations(self):
        enumerated = {}
        direct = set()
        for binary, pattern in re.findall(
            r"\(binary\(([A-Za-z0-9_-]+)\)\s*&\s*test\(/(.*?)/\)\)", self.medium
        ):
            inline = re.fullmatch(r"\^((?:[A-Za-z0-9_:]+::)?)\(([^)]*)\)::", pattern)
            if inline:
                key = (binary, inline[1].removesuffix("::"))
                enumerated.setdefault(key, set()).update(inline[2].split("|"))
            top = re.fullmatch(r"\^((?:[A-Za-z0-9_:]+::)?)\[\^:\]\+\$", pattern)
            if top:
                direct.add((binary, top[1].removesuffix("::")))
        return enumerated, direct

    def test_every_module_with_a_process_or_socket_is_declared_medium(self) -> None:
        """A module is covered in its own binary, including crate-root tests."""
        enumerated, direct = self.medium_declarations()
        declared = set(enumerated) | direct
        for crate, path, _, _ in flagged_sources():
            with self.subTest(module=f"{crate}::{path}"):
                self.assertIn((crate.replace("-", "_"), path), declared)

    def test_every_inline_test_module_is_named_in_its_term(self) -> None:
        enumerated, direct = self.medium_declarations()
        remedy = "перезапустите `python3 scripts/ci/size-filters.py --write`"
        for crate, path, modules, top_level in flagged_sources():
            key = (crate.replace("-", "_"), path)
            with self.subTest(module=f"{crate}::{path}"):
                self.assertEqual(sorted(modules - enumerated.get(key, set())), [], remedy)
                if top_level:
                    self.assertIn(key, direct, remedy)


if __name__ == "__main__":
    unittest.main()
