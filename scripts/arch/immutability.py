#!/usr/bin/env python3
"""Что случилось с продуктовыми записями базы: замена, объяснение или подмена.

Продуктовая запись — обещание, на которое кто-то снаружи уже опёрся, и менять
его молча нельзя. Но виды обещаний живут по-разному, поэтому и дисциплины две.

Решение говорит, что было выбрано на его дату. Его не правят: заводят преемника,
а старому ставят `superseded` и имя заменившего. Переход `planned` в `active` и
отметка о реализации сюда не относятся: они говорят не о решении, а о мире
вокруг него, появляются позже самого решения и записываются атомарно.

Инвариант и контракт описывают действующее правило, а правило со временем
уточняется. Их править можно — но только вместе с решением, которое этой же
правкой заводится и объясняет, почему правило меняется. Инвариант, изменившийся
без нового основания, и есть тихая смена обещания.

Процессная запись под это не подпадает: её и заводят, чтобы перестроить в тот
день, когда разрабатывать стало неудобно.

Сторона берётся из базы, а не из рабочего дерева: иначе правку продуктового
правила достаточно было бы прикрыть переводом его в процессные.

Usage:
    immutability.py --base origin/main
    immutability.py            # база по умолчанию origin/main
"""

from __future__ import annotations

import argparse
import ast
import importlib.util
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "arch_registry_for_immutability", Path(__file__).resolve().parent / "registry.py"
)
_REGISTRY = importlib.util.module_from_spec(_SPEC)
# До exec_module: `dataclass` внутри registry.py ищет свой модуль в sys.modules,
# и без записи там разбор падает на определении класса.
sys.modules[_SPEC.name] = _REGISTRY
_SPEC.loader.exec_module(_REGISTRY)

ARCH_PREFIX = "arch/"
# Допустимые отметки трогают ровно две атомарные пары полей.
SUPERSESSION_FIELDS = ("status", "superseded-by")
# А это — всё, что можно проставить принятой записи, не переписав её.
#
# `realized` держит адрес свидетельства, что решение построено. Свидетельство
# появляется после решения — иначе решение принималось бы задним числом, — и
# запретить его записывать значит запретить реестру знать, что построено.
# Рассуждением оно не является, поэтому под запрет на правку не подпадает; что
# названный адрес существует, сторожит отдельное правило.
REALIZATION_FIELDS = ("status", "realized")
RECORDABLE_FIELDS = frozenset(SUPERSESSION_FIELDS + REALIZATION_FIELDS)


@dataclass(frozen=True)
class Verdict:
    offenders: tuple[str, ...]
    compared: int

    def report(self) -> str:
        head = f"сверено продуктовых записей: {self.compared}"
        if not self.offenders:
            return head + "\nправок принятых записей нет"
        return head + "\n" + "\n".join(f"  {line}" for line in self.offenders)


@dataclass(frozen=True)
class IntroducedRecord:
    kind: str
    path: str
    props: dict


def _git(repo: Path, *args: str) -> str:
    done = subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, text=True, check=True
    )
    return done.stdout


def _records_at(repo: Path, ref: str) -> dict[str, str]:
    """Пути и содержимое записей реестра в указанной ревизии."""
    listing = _git(repo, "ls-tree", "-r", "--name-only", ref, "--", ARCH_PREFIX).split()
    found = {}
    for path in listing:
        if not path.endswith(".md") or path.endswith("index.md") or path.endswith("README.md"):
            continue
        found[path] = _git(repo, "show", f"{ref}:{path}")
    return found


def _split(text: str) -> tuple[dict, str]:
    try:
        return _REGISTRY.parse_front_matter(text)
    except ValueError:
        return {}, text


def _is_recordable_only(before: str, after: str) -> bool:
    """Правка сводится к простановке отметки и ничему больше."""
    old_props, old_body = _split(before)
    new_props, new_body = _split(after)
    if old_body != new_body:
        return False
    if set(old_props) != set(new_props):
        return False
    changed = {key for key in old_props if old_props[key] != new_props[key]}
    if not changed or any(key not in RECORDABLE_FIELDS for key in changed):
        return False

    if changed == set(SUPERSESSION_FIELDS):
        return (
            old_props["status"] in {"planned", "active"}
            and new_props["status"] == "superseded"
            and not old_props["superseded-by"]
            and bool(new_props["superseded-by"])
        )

    if changed == set(REALIZATION_FIELDS):
        return (
            old_props["status"] == "planned"
            and new_props["status"] == "active"
            and not old_props["realized"]
            and bool(new_props["realized"])
        )

    return False


def _records_introduced(repo: Path, base: dict[str, str]) -> dict[str, IntroducedRecord]:
    """Новые записи вместе с видом и props, нужными для проверки основания."""
    known = {
        props["id"]
        for text in base.values()
        if (props := _split(text)[0]).get("id")
    }
    introduced: dict[str, IntroducedRecord] = {}
    for directory, kind in (
        ("decisions", "decision"),
        ("invariants", "invariant"),
        ("contracts", "contract"),
    ):
        for path in sorted((repo / "arch" / directory).glob("*.md")):
            props, _ = _split(path.read_text(encoding="utf-8"))
            identifier = props.get("id")
            if identifier and identifier not in known:
                introduced[identifier] = IntroducedRecord(
                    kind=kind,
                    path=path.relative_to(repo).as_posix(),
                    props=props,
                )
    return introduced


def _python_defines(source: str, name: str) -> bool:
    try:
        tree = ast.parse(source)
    except SyntaxError:
        return False
    return any(
        isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == name
        for node in ast.walk(tree)
    )


def _rust_quoted_end(source: str, start: int, quote: str, multiline: bool) -> int | None:
    """End of a quoted literal, respecting escapes and its newline rule."""
    index = start + 1
    while index < len(source):
        if source[index] == "\\":
            index += 2
            continue
        if source[index] == quote:
            return index + 1
        if source[index] == "\n" and not multiline:
            return None
        index += 1
    return len(source) if multiline else None


def _rust_raw_string_end(source: str, start: int) -> int | None:
    """End of a Rust raw or raw-byte string, including its prefix and suffix."""
    index = start
    if source.startswith("br", index):
        index += 2
    elif source.startswith("r", index):
        index += 1
    else:
        return None
    hashes = 0
    while index < len(source) and source[index] == "#":
        hashes += 1
        index += 1
    if index == len(source) or source[index] != '"':
        return None
    closing = '"' + "#" * hashes
    end = source.find(closing, index + 1)
    return len(source) if end == -1 else end + len(closing)


def _rust_tokens(source: str) -> list[str]:
    """Tokenize enough Rust to recognize a real attributed test definition."""
    tokens: list[str] = []
    index = 0
    while index < len(source):
        if source[index].isspace():
            index += 1
            continue
        if source.startswith("//", index):
            end = source.find("\n", index)
            index = len(source) if end == -1 else end
            continue
        if source.startswith("/*", index):
            index += 2
            depth = 1
            while index < len(source) and depth:
                if source.startswith("/*", index):
                    depth += 1
                    index += 2
                elif source.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
            continue
        raw_end = _rust_raw_string_end(source, index)
        if raw_end is not None:
            index = raw_end
            continue
        if source[index] == '"':
            index = _rust_quoted_end(source, index, '"', multiline=True)
            continue
        if source.startswith('b"', index):
            index = _rust_quoted_end(source, index + 1, '"', multiline=True)
            continue
        if source[index] == "'":
            char_end = _rust_quoted_end(source, index, "'", multiline=False)
            if char_end is not None:
                index = char_end
                continue
        if source.startswith("b'", index):
            char_end = _rust_quoted_end(source, index + 1, "'", multiline=False)
            if char_end is not None:
                index = char_end
                continue
        if source[index].isalpha() or source[index] == "_":
            end = index + 1
            while end < len(source) and (source[end].isalnum() or source[end] == "_"):
                end += 1
            tokens.append(source[index:end])
            index = end
            continue
        if source.startswith("::", index):
            tokens.append("::")
            index += 2
            continue
        tokens.append(source[index])
        index += 1
    return tokens


def _rust_balanced(tokens: list[str], index: int, opening: str, closing: str) -> int | None:
    """Return the token after a balanced delimiter pair beginning at index."""
    if index == len(tokens) or tokens[index] != opening:
        return None
    depth = 1
    braces = 0
    index += 1
    while index < len(tokens) and depth:
        token = tokens[index]
        if opening == "<" and token == "{":
            braces += 1
        elif opening == "<" and token == "}" and braces:
            braces -= 1
        elif token == opening and braces == 0:
            depth += 1
        elif token == closing and braces == 0:
            depth -= 1
        index += 1
    return index if depth == 0 else None


def _rust_attribute(tokens: list[str], index: int) -> tuple[bool, int] | None:
    """Classify one `#[...]` attribute and return the first token after it."""
    if tokens[index : index + 2] != ["#", "["]:
        return None
    end = _rust_balanced(tokens, index + 1, "[", "]")
    if end is None:
        return None
    path: list[str] = []
    cursor = index + 2
    while cursor < end - 1 and (not path or tokens[cursor - 1] == "::"):
        if not (tokens[cursor][0].isalpha() or tokens[cursor][0] == "_"):
            break
        path.append(tokens[cursor])
        cursor += 1
        if cursor < end - 1 and tokens[cursor] == "::":
            cursor += 1
            continue
        break
    return (bool(path) and path[-1] == "test", end)


def _rust_after_qualifiers(tokens: list[str], index: int) -> int:
    """Skip the qualifiers Rust permits before a free or associated function."""
    while index < len(tokens):
        if tokens[index] == "pub":
            index += 1
            if index < len(tokens) and tokens[index] == "(":
                after = _rust_balanced(tokens, index, "(", ")")
                if after is None:
                    return len(tokens)
                index = after
            continue
        if tokens[index] in {"async", "unsafe", "const"}:
            index += 1
            continue
        if tokens[index] == "extern":
            index += 1
            continue
        break
    return index


def _rust_signature_has_body(tokens: list[str], index: int) -> bool:
    """A signature ends in `{` only at its outer delimiter depth, not in a type."""
    if index < len(tokens) and tokens[index] == "<":
        after = _rust_balanced(tokens, index, "<", ">")
        if after is None:
            return False
        index = after
    after_parameters = _rust_balanced(tokens, index, "(", ")")
    if after_parameters is None:
        return False
    index = after_parameters
    parens = brackets = angles = braces = 0
    while index < len(tokens):
        token = tokens[index]
        if token == "(":
            parens += 1
        elif token == ")" and parens:
            parens -= 1
        elif token == "[":
            brackets += 1
        elif token == "]" and brackets:
            brackets -= 1
        elif token == "<" and braces == 0:
            angles += 1
        elif token == ">" and braces == 0 and angles:
            angles -= 1
        elif token == "{":
            if parens == brackets == angles == braces == 0:
                return True
            braces += 1
        elif token == "}" and braces:
            braces -= 1
        elif token == ";" and parens == brackets == angles == braces == 0:
            return False
        index += 1
    return False


def _rust_test_function_has_body(source: str, name: str) -> bool:
    """Find an exact attributed Rust test definition, never a declaration."""
    tokens = _rust_tokens(source)
    for index, token in enumerate(tokens):
        if token != "#":
            continue
        attribute = _rust_attribute(tokens, index)
        if attribute is None or not attribute[0]:
            continue
        cursor = attribute[1]
        while (next_attribute := _rust_attribute(tokens, cursor)) is not None:
            cursor = next_attribute[1]
        cursor = _rust_after_qualifiers(tokens, cursor)
        if tokens[cursor : cursor + 2] != ["fn", name]:
            continue
        if _rust_signature_has_body(tokens, cursor + 2):
            return True
    return False


def _evidence_resolves(repo: Path, evidence: object) -> bool:
    """The named evidence belongs to this repository and defines a test function."""
    if not isinstance(evidence, str):
        return False
    relative, separator, name = evidence.partition("::")
    if separator != "::" or not relative or not name:
        return False
    root = repo.resolve()
    target = (root / relative).resolve()
    if not target.is_relative_to(root) or not target.is_file():
        return False
    source = target.read_text(encoding="utf-8")
    if target.suffix == ".py":
        return _python_defines(source, name)
    if target.suffix == ".rs":
        return _rust_test_function_has_body(source, name)
    return False


def _ground_error(repo: Path, ground: IntroducedRecord) -> str | None:
    """A new product-rule ground must be an implemented product decision."""
    if ground.kind != "decision":
        return f"основание {ground.path} не является decision"
    if ground.props.get("status") != "active":
        return f"решение {ground.path} имеет status {ground.props.get('status')!r}, не active"
    if ground.props.get("governs") != "product":
        return f"решение {ground.path} governs {ground.props.get('governs')!r}, не product"
    evidence = ground.props.get("realized")
    if not evidence:
        return f"решение {ground.path} не имеет realized evidence"
    if not _evidence_resolves(repo, evidence):
        return f"realized evidence {evidence!r} решения {ground.path} не разрешается"
    return None


def inspect(repo: Path, base_ref: str) -> Verdict:
    """Сверить принятые продуктовые записи базы с рабочим деревом."""
    base = _records_at(repo, base_ref)
    introduced = _records_introduced(repo, base)
    offenders: list[str] = []
    compared = 0

    for path, before in sorted(base.items()):
        props, _ = _split(before)
        if props.get("governs") != "product":
            continue
        compared += 1

        current = repo / path
        if not current.is_file():
            offenders.append(f"{path}: продуктовая запись удалена, а не заменена")
            continue

        after = current.read_text(encoding="utf-8")
        if after == before:
            continue

        if path.startswith("arch/decisions/"):
            if not _is_recordable_only(before, after):
                offenders.append(f"{path}: продуктовое решение отредактировано, а не заменено")
            continue

        # Инвариант и контракт: правка законна, если этой же правкой заведено
        # решение, на которое запись теперь и ссылается. Существующее основание
        # не годится — оно писалось раньше и этой перемены не предвидело.
        ground = _split(after)[0].get("decision")
        introduced_ground = introduced.get(ground)
        if introduced_ground is None:
            offenders.append(
                f"{path}: продуктовое правило изменено без нового решения о причине"
            )
            continue
        if error := _ground_error(repo, introduced_ground):
            offenders.append(f"{path}: продуктовое правило изменено, но {error}")

    return Verdict(tuple(offenders), compared)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", default="origin/main")
    parser.add_argument("--repo", default=".")
    arguments = parser.parse_args(argv)

    verdict = inspect(Path(arguments.repo).resolve(), arguments.base)
    print(verdict.report())
    return 1 if verdict.offenders else 0


if __name__ == "__main__":
    raise SystemExit(main())
