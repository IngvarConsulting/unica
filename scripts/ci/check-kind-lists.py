#!/usr/bin/env python3
"""Списки видов объектов собираются из объявлений, а не правятся строкой.

Каждый вид объявлен один раз: файл `crates/unica-address/src/kinds/<вид>.rs`.
Страж отклоняет второе объявление того же написания и любой список
в исходниках крейтов, который перечисляет виды корня почти целиком
(`LIST_SHARE` объявленных и больше): такой список дописывали бы при каждом
новом виде, и два изменения с разными видами конфликтовали бы в нём.
Список видов строится из `unica_address::kinds`. Подмножество видов по
признаку — права роли, модули менеджера — стражем не считается.

Список — самое узкое выражение в скобках, которое называет столько видов.
Тело функции, модуля, `impl` и `trait` списком не бывает: виды соседних
функций и тестов в нём не складываются.
Вид назван строковым литералом своего написания (`"Queue"`) или
идентификатором варианта (`NodeKind::Queue`, вариант своего перечисления).
"""
from __future__ import annotations

from dataclasses import dataclass
import math
from pathlib import Path
import re
import subprocess
import sys
from typing import Iterator

ROOT = Path(__file__).resolve().parents[2]
DECLARATIONS = "crates/unica-address/src/kinds"
LIST_SHARE = 0.75
TAG = re.compile(r'KindDeclaration::new\("([^"]+)"\)')
RAW = re.compile(r'[bc]?r(#*)"')
CHAR = re.compile(r"'(?:\\.|[^\\'\n])'")
IDENT = re.compile(r"\w+")
ITEMS = {"fn", "mod", "impl", "trait"}
# Начало другого элемента или выражения: `fn` перед ним был типом, а не телом.
NOT_ITEMS = {"enum", "struct", "union", "const", "static", "let", "match", "type"}


@dataclass(frozen=True)
class Kind:
    tag: str
    variant: str
    root: bool


def declared(root: Path) -> tuple[list[Kind], list[str]]:
    kinds: list[Kind] = []
    errors: list[str] = []
    owners: dict[str, str] = {}
    for path in sorted((root / DECLARATIONS).glob("*.rs")):
        text = path.read_text(encoding="utf-8")
        tags = TAG.findall(text)
        relative = path.relative_to(root).as_posix()
        if len(tags) != 1:
            errors.append(f"{relative}: declare exactly one kind with KindDeclaration::new")
            continue
        variant = "".join(word.capitalize() for word in path.stem.split("_"))
        for spelling in {tags[0], variant}:
            if spelling in owners and owners[spelling] != relative:
                errors.append(f"{relative}: `{spelling}` is already declared by {owners[spelling]}")
            owners[spelling] = relative
        kinds.append(Kind(tags[0], variant, "root: Some(" in text))
    return kinds, errors


def sources(root: Path) -> list[str]:
    listing = subprocess.run(
        ["git", "ls-files", "-z", "crates/*.rs"], cwd=root, check=True, capture_output=True
    ).stdout.decode("utf-8")
    return sorted(
        name for name in listing.split("\0") if name and not name.startswith(DECLARATIONS + "/")
    )


def tokens(text: str) -> Iterator[tuple[int, str, str]]:
    """Идентификаторы, строковые литералы и скобки Rust с номерами строк;
    комментарии пропускаются."""
    index, line, size = 0, 1, len(text)
    while index < size:
        char = text[index]
        if char == "\n":
            line += 1
            index += 1
        elif text.startswith("//", index):
            end = text.find("\n", index)
            index = size if end < 0 else end
        elif text.startswith("/*", index):
            depth, index = 1, index + 2
            while index < size and depth:
                if text.startswith("/*", index):
                    depth, index = depth + 1, index + 2
                elif text.startswith("*/", index):
                    depth, index = depth - 1, index + 2
                else:
                    line += text[index] == "\n"
                    index += 1
        elif char in "bcr" and (raw := RAW.match(text, index)):
            hashes = raw.group(1)
            start = raw.end()
            end = text.index('"' + hashes, start)
            yield line, "string", text[start:end]
            line += text.count("\n", index, end)
            index = end + 1 + len(hashes)
        elif char == '"':
            end = index + 1
            while text[end] != '"':
                end += 2 if text[end] == "\\" else 1
            yield line, "string", text[index + 1:end]
            line += text.count("\n", index, end)
            index = end + 1
        elif char == "'" and (literal := CHAR.match(text, index)):
            index = literal.end()
        elif char.isalpha() or char == "_":
            end = IDENT.match(text, index).end()
            yield line, "ident", text[index:end]
            index = end
        elif char in "([{":
            yield line, "open", char
            index += 1
        elif char in ")]}":
            yield line, "close", char
            index += 1
        else:
            index += 1


def lists(text: str, kinds: list[Kind]) -> list[tuple[int, set[str]]]:
    """Самые узкие выражения в скобках, которые называют `LIST_SHARE` видов
    корня и больше: строка открывающей скобки и названные виды."""
    roots = [kind for kind in kinds if kind.root]
    threshold = max(2, math.ceil(len(roots) * LIST_SHARE))
    strings = {kind.tag: kind.tag for kind in roots}
    idents = {kind.variant: kind.tag for kind in roots}
    found: list[tuple[int, set[str]]] = []
    # Открытая скобка: строка, названные внутри виды, есть ли внутри список,
    # тело ли это элемента. Тело `fn`, `mod`, `impl`, `trait` держит
    # независимые элементы: виды соседних функций списком не складываются.
    stack: list[tuple[int, set[str], bool, bool]] = [(1, set(), False, True)]
    item_at: int | None = None
    for line, kind, value in tokens(text):
        if kind == "ident" and value in ITEMS:
            item_at = len(stack)
        elif kind == "ident" and value in NOT_ITEMS:
            item_at = None
        elif kind == "open":
            body = value == "{" and item_at == len(stack)
            if body:
                item_at = None
            stack.append((line, set(), False, body))
        elif kind == "close":
            if len(stack) == 1:
                continue
            start, named, inner, body = stack.pop()
            listed = not body and not inner and len(named) >= threshold
            if listed:
                found.append((start, named))
            if body:
                continue
            outer_line, outer_named, outer_inner, outer_body = stack.pop()
            stack.append((outer_line, outer_named | named, outer_inner or inner or listed, outer_body))
        else:
            tag = strings.get(value) if kind == "string" else idents.get(value)
            if tag:
                stack[-1][1].add(tag)
    return found


def check(root: Path = ROOT) -> list[str]:
    kinds, errors = declared(root)
    roots = sum(kind.root for kind in kinds)
    threshold = max(2, math.ceil(roots * LIST_SHARE))
    words = {kind.tag for kind in kinds if kind.root} | {kind.variant for kind in kinds if kind.root}
    mention = re.compile(r"\b(?:" + "|".join(map(re.escape, sorted(words))) + r")\b")
    for name in sources(root):
        text = (root / name).read_text(encoding="utf-8")
        # Файл, где видов меньше порога, списка не держит: лексер ему не нужен.
        if len(set(mention.findall(text))) < threshold:
            continue
        for line, named in lists(text, kinds):
            errors.append(
                f"{name}:{line}: the list names {len(named)} of {roots} root kinds; "
                f"build it from unica_address::kinds instead of listing kinds"
            )
    return errors


def main() -> int:
    errors = check()
    for error in errors:
        print(error, file=sys.stderr)
    return int(bool(errors))


if __name__ == "__main__":
    raise SystemExit(main())
