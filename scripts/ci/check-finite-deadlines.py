#!/usr/bin/env python3
"""Новый конечный срок операции не заводится вне списка разрешённых мест.

Правило `INV.RUNTIME.TIMERS-PRESERVE-WORK`: производственный путь операции не
запускает новый конечный `ProviderDeadline` ни под своим именем, ни под
псевдонимом из `use`. Конечный срок заводят `from_budget`, `new` и
`from_started_at`. Список ниже называет файлы, где это разрешено, и число
таких вызовов в каждом. Новый вызов где угодно или лишний в названном файле —
нарушение, как и пропавший.

Страж разбирает Rust-синтаксис и не раскрывает макросы: вызов внутри
аргументов макроса не виден, как и компилятору до раскрытия. Код только для
тестов пропускается: файл, элемент модуля, метод `impl` или оператор под
`cfg(test)`, `cfg(all(test, ..))` или `cfg(feature = "receipt-ledger-test-support")`.
Условие вроде `cfg(any(test, ..))` собирается и вне тестов и остаётся
производственным.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

from tree_sitter import Language, Parser
import tree_sitter_rust

RUST_LANGUAGE = Language(tree_sitter_rust.language())
OWNER = "ProviderDeadline"
FINITE_CONSTRUCTORS = {"from_budget", "new", "from_started_at"}

# Места, где конечный срок разрешён: шаговая граница, срок, заданный
# пользователем явно, переходник уже существующего срока вызывающего, путь,
# куда не доходит провод v0.13, или поиск, перенесённый в #1254. У переходника
# существующего срока есть тест, который отвергает его перевзвод как бюджета.
ALLOWED: dict[str, tuple[int, str]] = {
    "crates/unica-code-intelligence/src/coordinator.rs": (2, "search and provider reads: #1254"),
    "crates/unica-diagnostics/src/coordinator.rs": (
        2, "explicit analyze timeout; resident actions of the retired code.diagnostics"),
    "crates/unica-application/src/application/mod.rs": (
        1, "PUBLIC_INVOCATION_DEADLINE: legacy v0.12 surface, unreachable from the v0.13 wire"),
    "crates/unica-coder/src/infrastructure/application_ports.rs": (
        1, "NATIVE_TYPED_INVOCATION_DEADLINE: legacy v0.12 surface"),
    "crates/unica-lsp/src/host.rs": (
        1, "a symbol call carries the editor request's own cutoff, set by the LSP lane, to the provider; "
           "a_symbol_call_gives_the_provider_the_requests_own_deadline rejects rearming"),
    "crates/unica-daemon/src/infrastructure/daemon/invocation_service.rs": (
        1, "saved apply adapts the existing caller cutoff; "
           "saved_apply_adapter_preserves_the_existing_cutoff_and_absence rejects rearming"),
    "crates/unica-execution/src/receipt_runtime.rs": (27, "per-step Task store and lifecycle-link I/O bounds"),
}

# Элементы модуля или блока, которых атрибут `cfg` убирает целиком. В `impl`
# так убирается только метод; элементы `trait` проверяются всегда.
GATED_ITEMS = {
    "function_item", "mod_item", "impl_item", "const_item", "static_item",
    "struct_item", "use_declaration", "macro_definition", "macro_invocation",
}
# Операторы блока, которых `cfg` убирает целиком: `let` и выражение с `if`,
# вызовом функции или метода либо макросом.
GATED_EXPRESSIONS = {"if_expression", "call_expression", "macro_invocation"}
TEST_CONDITIONS = re.compile(r'test|all\(test,.*|feature="receipt-ledger-test-support"', re.S)
NOT_CODE = {"line_comment", "block_comment"}


def condition(attribute) -> str | None:
    """Условие `cfg(...)` атрибута без пробелов, или `None` для другого атрибута."""
    inner = next((child for child in attribute.named_children if child.type == "attribute"), None)
    if inner is None or not inner.named_children or inner.named_children[0].text != b"cfg":
        return None
    arguments = inner.child_by_field_name("arguments")
    if arguments is None:
        return None
    return re.sub(rb"\s+", b"", arguments.text[1:-1]).decode("utf-8")


def is_test_only(attributes) -> bool:
    return any(TEST_CONDITIONS.fullmatch(condition(attribute) or "") for attribute in attributes)


def attributes_above(node) -> list:
    """Внешние атрибуты над узлом: в tree-sitter это предшествующие соседи."""
    found = []
    sibling = node.prev_named_sibling
    while sibling is not None and sibling.type in {"attribute_item", *NOT_CODE}:
        if sibling.type == "attribute_item":
            found.append(sibling)
        sibling = sibling.prev_named_sibling
    return found


def gateable(node) -> bool:
    """Узел, который атрибут `cfg` над ним убирает из сборки целиком."""
    parent = node.parent
    if parent is None:
        return False
    if parent.type == "declaration_list" and parent.parent is not None and parent.parent.type in {
            "impl_item", "trait_item"}:
        return node.type == "function_item" and parent.parent.type == "impl_item"
    if parent.type in {"source_file", "declaration_list", "block"} and node.type in GATED_ITEMS:
        return True
    if parent.type != "block":
        return False
    if node.type == "let_declaration":
        return True
    if node.type == "expression_statement":
        node = next((child for child in node.named_children if child.type not in NOT_CODE), None)
        return node is not None and node.type in GATED_EXPRESSIONS
    return node.type in GATED_EXPRESSIONS


def gated(node) -> bool:
    return gateable(node) and is_test_only(attributes_above(node))


def path_segments(node) -> list[str]:
    """Сегменты пути `a::b::c` по порядку."""
    segments = []
    while node is not None and node.type == "scoped_identifier":
        name = node.child_by_field_name("name")
        segments.append(name.text.decode("utf-8") if name is not None else "")
        node = node.child_by_field_name("path")
    if node is not None:
        segments.append(node.text.decode("utf-8"))
    return segments[::-1]


def finite_deadline_calls(source: str) -> int:
    """Вызовы конечного конструктора `ProviderDeadline` в производственном коде."""
    tree = Parser(RUST_LANGUAGE).parse(source.encode("utf-8"))
    root = tree.root_node
    if is_test_only([child for child in root.named_children if child.type == "inner_attribute_item"]):
        return 0
    aliases: set[str] = set()
    calls = 0
    pending = [root]
    while pending:
        node = pending.pop()
        if gated(node):
            continue
        if node.type == "use_as_clause":
            path = node.child_by_field_name("path")
            alias = node.child_by_field_name("alias")
            if path is not None and alias is not None and path_segments(path)[-1] == OWNER:
                aliases.add(alias.text.decode("utf-8"))
        elif node.type == "scoped_identifier" and not inside_path_or_use(node):
            segments = path_segments(node)
            if len(segments) >= 2 and segments[-1] in FINITE_CONSTRUCTORS and (
                    segments[-2] == OWNER or segments[-2] in aliases):
                calls += 1
        pending.extend(reversed(node.named_children))
    return calls


def inside_path_or_use(node) -> bool:
    """Часть более длинного пути или путь импорта: не выражение-путь."""
    parent = node.parent
    if parent is not None and parent.type == "scoped_identifier" and parent.child_by_field_name("path") == node:
        return True
    while parent is not None:
        if parent.type in {"use_declaration", "attribute_item", "inner_attribute_item"}:
            return True
        parent = parent.parent
    return False


def is_test_source(relative: str) -> bool:
    """Файлы, которые собираются только в тесты или в тестовую обвязку журнала."""
    name = relative.rsplit("/", 1)[-1]
    return (name == "tests.rs" or name.endswith("_tests.rs")
            or name in {"test_support.rs", "test_control.rs", "testing.rs"}
            or "/tests/" in relative or "receipt_scenario_v5" in relative)


def found_calls(root: Path) -> dict[str, int]:
    """Производственные файлы `crates/*/src` с вызовами конечного конструктора."""
    found = {}
    for path in sorted((root / "crates").glob("*/src/**/*.rs")):
        relative = path.relative_to(root).as_posix()
        if is_test_source(relative):
            continue
        source = path.read_text(encoding="utf-8")
        # Вызов возможен только через `ProviderDeadline` или его псевдоним из
        # `use` того же файла: без этого имени разбирать файл нечего.
        if OWNER not in source:
            continue
        calls = finite_deadline_calls(source)
        if calls:
            found[relative] = calls
    return found


def violations(found: dict[str, int]) -> list[str]:
    allowed = {path: calls for path, (calls, _) in ALLOWED.items()}
    lines = []
    for path in sorted(found.keys() | allowed.keys()):
        actual, expected = found.get(path, 0), allowed.get(path, 0)
        if actual != expected:
            lines.append(f"{path}: {actual} finite ProviderDeadline constructor call(s), allowed {expected}")
    return lines


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args(argv)
    lines = violations(found_calls(args.root))
    for line in lines:
        print(line, file=sys.stderr)
    if lines:
        print("a production path starts a finite operation deadline outside the per-step allow-list "
              "(scripts/ci/check-finite-deadlines.py, INV.RUNTIME.TIMERS-PRESERVE-WORK)", file=sys.stderr)
    return 1 if lines else 0


if __name__ == "__main__":
    sys.exit(main())
