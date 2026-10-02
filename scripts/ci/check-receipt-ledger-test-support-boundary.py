#!/usr/bin/env python3
"""Проверка размещения атрибутов `receipt-ledger-test-support` в Rust.

`cfg` и `cfg_attr` могут включать целые items, например функцию или модуль,
но не отдельные операторы, выражения, аргументы, поля или ветви `match`.
Форма `not` с этой feature запрещена и на целых items.

Страж разбирает Rust-синтаксис, включая записанные тела `macro_rules!`.
Он не раскрывает макросы и не доказывает эквивалентность сборок.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from tree_sitter import Language, Parser
import tree_sitter_rust

FEATURE = "receipt-ledger-test-support"
SOURCE_ROOT = Path("crates/unica-coder/src")
RUST_LANGUAGE = Language(tree_sitter_rust.language())
ITEM_TYPES = {
    "mod_item", "use_declaration", "function_item", "function_signature_item",
    "struct_item", "enum_item", "impl_item", "trait_item", "type_item",
    "const_item", "static_item", "macro_definition",
}
COMMENTS = {"line_comment", "block_comment"}


def uses_feature(node) -> bool:
    children = [child for child in node.children if child.type not in COMMENTS]
    for name, equals, value in zip(children, children[1:], children[2:]):
        if name.text == b"feature" and equals.text == b"=":
            if any(child.type == "string_content" and child.text == FEATURE.encode()
                   for child in value.named_children):
                return True
    return any(uses_feature(child) for child in node.named_children
               if child.type == "token_tree")


def negates_feature(node) -> bool:
    children = [child for child in node.children if child.type not in COMMENTS]
    for name, arguments in zip(children, children[1:]):
        if name.text == b"not" and arguments.type == "token_tree" and uses_feature(arguments):
            return True
    return any(negates_feature(child) for child in node.named_children
               if child.type == "token_tree")


def offenders(path: Path, source: str, line_offset: int = 0) -> list[str]:
    found = []
    tree = Parser(RUST_LANGUAGE).parse(source.encode("utf-8"))
    pending = [tree.root_node]
    while pending:
        node = pending.pop()
        pending.extend(reversed(node.named_children))
        if node.type == "macro_rule":
            body = node.child_by_field_name("right")
            if body is not None:
                # Inspect the written expansion body without expanding metavariables.
                found.extend(offenders(
                    path, body.text[1:-1].decode("utf-8"),
                    line_offset + body.start_point.row,
                ))
        if node.type not in {"attribute_item", "inner_attribute_item"}:
            continue
        attribute = node.named_children[0]
        if not attribute.named_children or attribute.named_children[0].text not in (b"cfg", b"cfg_attr"):
            continue
        arguments = attribute.child_by_field_name("arguments")
        if arguments is None or not uses_feature(arguments):
            continue
        line = line_offset + node.start_point.row + 1
        if negates_feature(arguments):
            found.append(f"{path.as_posix()}:{line}: `not(feature = ...)` is forbidden")
            continue
        if node.type == "inner_attribute_item":
            target = node.parent
            if target.type == "declaration_list":
                target = target.parent
        else:
            target = node.next_named_sibling
            while target is not None and target.type in COMMENTS | {"attribute_item"}:
                target = target.next_named_sibling
        if target is None or target.type not in ITEM_TYPES | {"source_file"}:
            gated = "" if target is None else target.text.decode("utf-8").strip()[:60]
            found.append(
                f"{path.as_posix()}:{line}: the feature gates `{gated}`, not an item"
            )
    return found


def scan(root: Path) -> list[str]:
    found = []
    for path in sorted((root / SOURCE_ROOT).rglob("*.rs")):
        found.extend(offenders(path.relative_to(root), path.read_text(encoding="utf-8")))
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    arguments = parser.parse_args()
    found = scan(arguments.root)
    for line in found:
        print(line)
    if found:
        print(f"{len(found)} feature-gated non-items", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
