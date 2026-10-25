#!/usr/bin/env python3
"""Сверка реестра ресурсных ограничений `arch/resource-limits.json` с кодом.

Число предела живёт в коде. Реестр повторяет выражение константы и сверяется
с ним: расхождение, неучтённая константа-предел или устаревшая запись дают
ошибку. Константу-предел страж узнаёт по имени (`LIMIT_NAME`); предел,
записанный литералом, вносится в реестр вручную и сверяется по числу вхождений.
`--scaffold` печатает заготовки записей для неучтённых констант.
"""

from __future__ import annotations

import argparse
import ast
import functools
import json
import re
import sys
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[2]
REGISTRY = Path("arch/resource-limits.json")
SCHEMA = Path("arch/resource-limits.schema.json")

# Имя константы похоже на ресурсный предел, срок, окно, порцию или ёмкость.
LIMIT_NAME = re.compile(
    r"(^MAX_|_MAX_|_MAX$|LIMIT|BUDGET|_CAP$|_CAP_|QUOTA|TIMEOUT|DEADLINE|_TTL|^TTL_|TTL$"
    r"|THRESHOLD|_WINDOW$|CAPACITY|^RETAINED_|_BYTES$|_SECONDS$|_SECS$|_MS$|PAGE_SIZE|PAGE_BYTES|CHUNK"
    r"|RETRIES|RETRY|ATTEMPTS|GRACE|RETENTION|_INTERVAL$|_DELAY$|_DEPTH$|BOUND|_POLL$|STACK_SIZE)"
)
DURATION = re.compile(r"^(?:std::time::)?Duration::from_(secs|millis|mins)\((.+)\)$", re.S)
UNIT_MS = {"secs": 1000, "millis": 1, "mins": 60_000}
ITEM_KINDS = {"const_item", "static_item"}
CONTAINERS = {"impl_item": "impl", "trait_item": "trait", "function_item": "fn"}


class Constant(NamedTuple):
    file: str
    line: int
    symbol: str
    within: str | None
    type: str
    expr: str


class Source(NamedTuple):
    crate: str
    text: str
    identifiers: frozenset[str]
    constants: tuple[Constant, ...]


@functools.lru_cache(maxsize=1)
def rust_parser():
    import tree_sitter
    import tree_sitter_rust

    return tree_sitter.Parser(tree_sitter.Language(tree_sitter_rust.language()))


def module_dir(path: Path) -> Path:
    return path.parent if path.name in ("lib.rs", "main.rs", "mod.rs") else path.parent / path.stem


def cfg_predicate(text: str):
    """Предикат `#[cfg(...)]` деревом кортежей; `None` для прочих атрибутов."""
    match = re.fullmatch(r"#\[\s*cfg\s*\((.*)\)\s*\]", text, re.S)
    if not match:
        return None
    tokens = re.findall(r'"[^"]*"|[A-Za-z_][A-Za-z0-9_]*|[(),=]', match.group(1))
    position = 0

    def predicate():
        nonlocal position
        name = tokens[position]
        position += 1
        if position < len(tokens) and tokens[position] == "(":
            position += 1
            items = []
            while tokens[position] != ")":
                items.append(predicate())
                if tokens[position] == ",":
                    position += 1
            position += 1
            return (name, items)
        if position < len(tokens) and tokens[position] == "=":
            value = tokens[position + 1]
            position += 2
            return ("=", name, value.strip('"'))
        return ("flag", name)

    try:
        return predicate()
    except IndexError:
        return None


def test_only(node) -> bool:
    """Предикат выполняется только в сборке тестов: `test` или фича `*test-support`."""
    if node[0] == "flag":
        return node[1] == "test"
    if node[0] == "=":
        return node[1] == "feature" and node[2].endswith("test-support")
    name, items = node
    if name == "all":
        return any(test_only(item) for item in items)
    if name == "any":
        return bool(items) and all(test_only(item) for item in items)
    return False


def is_test_attribute(node) -> bool:
    if node.type != "attribute_item":
        return False
    predicate = cfg_predicate(node.text.decode())
    return predicate is not None and test_only(predicate)


def test_items(root_node):
    """Узлы, помеченные `#[cfg(test)]`: атрибут относится к следующему элементу."""
    found = []

    def walk(node):
        pending = False
        for child in node.children:
            if child.type == "attribute_item":
                pending = pending or is_test_attribute(child)
                continue
            if child.type in ("line_comment", "block_comment"):
                continue
            if pending:
                found.append(child)
                pending = False
                continue
            walk(child)

    walk(root_node)
    return found


def parse(root: Path, path: Path, test_files: set[Path]) -> Source:
    data = path.read_bytes()
    tree = rust_parser().parse(data)
    blank = bytearray(data)
    tests = test_items(tree.root_node)
    skipped = {(item.start_byte, item.end_byte) for item in tests}
    for item in tests:
        if item.type == "mod_item" and item.child_by_field_name("body") is None:
            name = item.child_by_field_name("name").text.decode()
            test_files.add(module_dir(path) / f"{name}.rs")
            test_files.add(module_dir(path) / name)
        for index in range(item.start_byte, item.end_byte):
            if blank[index] not in (0x0A, 0x0D):
                blank[index] = 0x20
    text = blank.decode("utf-8")
    rel = path.relative_to(root).as_posix()
    identifiers = set()
    constants = []

    def walk(node, within):
        if (node.start_byte, node.end_byte) in skipped:
            return
        if node.type == "identifier" and node.text.decode().isupper():
            identifiers.add(node.text.decode())
        if node.type in ITEM_KINDS:
            name = node.child_by_field_name("name").text.decode()
            value = node.child_by_field_name("value")
            kind = node.child_by_field_name("type")
            if value is not None and kind is not None and LIMIT_NAME.search(name):
                expr = value.text.decode().strip()
                # `pub const X: T = super::X;` переэкспортирует уже учтённый предел.
                if not normalize(expr).endswith(f"::{name}"):
                    constants.append(Constant(rel, node.start_point[0] + 1, name, within, kind.text.decode(), expr))
        if node.type in CONTAINERS:
            label = node.child_by_field_name("type") or node.child_by_field_name("name")
            within = f"{CONTAINERS[node.type]} {label.text.decode()}" if label is not None else within
        for child in node.children:
            walk(child, within)

    walk(tree.root_node, None)
    return Source(rel.split("/")[1], text, frozenset(identifiers), tuple(constants))


@functools.lru_cache(maxsize=4)
def sources(root: Path) -> dict[str, Source]:
    test_files: set[Path] = set()
    parsed = {}
    for path in sorted(root.glob("crates/*/src/**/*.rs")):
        if path.name == "tests.rs" or path.name.endswith("_tests.rs") or "/tests/" in path.relative_to(root).as_posix():
            continue
        parsed[path] = parse(root, path, test_files)
    # `#[cfg(test)] mod x;` makes the whole file `x.rs` and its directory test-only.
    return {
        path.relative_to(root).as_posix(): source
        for path, source in parsed.items()
        if not any(path == item or item in path.parents for item in test_files)
    }


def normalize(expr: str) -> str:
    return re.sub(r"\s+", "", expr)


def scan(root: Path) -> list[Constant]:
    return [constant for source in sources(root).values() for constant in source.constants]


def key(file: str, symbol: str, within: str | None) -> tuple[str, str, str | None]:
    return (file, symbol, within)


def evaluate(expr: str, unit: str, names: dict[str, int] | None = None) -> int | None:
    """Значение выражения в единице записи; `None`, если выражение не вычисляется."""
    text = expr.strip()
    duration = DURATION.match(text)
    if duration:
        if unit not in ("ms", "s"):
            return None
        inner = evaluate(duration.group(2), "count", names)
        if inner is None:
            return None
        millis = inner * UNIT_MS[duration.group(1)]
        return millis if unit == "ms" else (millis // 1000 if millis % 1000 == 0 else None)
    text = re.sub(r"(?<=\d)_(?=\d)", "", text)
    text = re.sub(r"(?<=\d)_?(?:usize|u64|u32|u16|u8|i64|i32)\b", "", text)
    text = re.sub(r"\bas\s+(?:usize|u64|u32|u16|u8|i64|i32)\b", "", text)
    text = re.sub(r"\b(?:crate|super|self|[a-z_][a-z0-9_]*)::", "", text)
    try:
        tree = ast.parse(text, mode="eval")
    except SyntaxError:
        return None

    def walk(node: ast.AST) -> int:
        if isinstance(node, ast.Expression):
            return walk(node.body)
        if isinstance(node, ast.Constant) and isinstance(node.value, int) and not isinstance(node.value, bool):
            return node.value
        if isinstance(node, ast.Name) and names and node.id in names:
            return names[node.id]
        if isinstance(node, ast.BinOp) and isinstance(node.op, (ast.Add, ast.Sub, ast.Mult, ast.FloorDiv, ast.LShift)):
            left, right = walk(node.left), walk(node.right)
            if isinstance(node.op, ast.Add):
                return left + right
            if isinstance(node.op, ast.Sub):
                return left - right
            if isinstance(node.op, ast.Mult):
                return left * right
            if isinstance(node.op, ast.LShift):
                return left << right
            return left // right
        raise ValueError(ast.dump(node))

    try:
        return walk(tree)
    except ValueError:
        return None


def literal_count(root: Path, file: str, literal: str) -> int:
    source = sources(root).get(file)
    return 0 if source is None else normalize(source.text).count(normalize(literal))


def users(root: Path, constant: Constant) -> list[str]:
    """Файлы продукта, где применяется именно эта константа.

    Файл со своим одноимённым пределом относится к своему. В крейте с несколькими
    одноимёнными пределами чужой файл засчитывается по пути модуля, в другом
    крейте — по имени крейта-владельца.
    """
    parsed = sources(root)
    owner = parsed[constant.file]
    owner_crate = owner.crate.replace("-", "_")
    module = Path(constant.file).stem if Path(constant.file).stem not in ("mod", "lib", "main") else Path(constant.file).parent.name
    same_named = [path for path, source in parsed.items() if any(item.symbol == constant.symbol for item in source.constants)]
    in_crate = [path for path in same_named if parsed[path].crate == owner.crate]
    found = {constant.file}
    for path, source in parsed.items():
        if path == constant.file or constant.symbol not in source.identifiers or path in same_named:
            continue
        if source.crate == owner.crate:
            if len(in_crate) == 1 or f"{module}::" in source.text:
                found.add(path)
        elif owner_crate in source.text:
            found.add(path)
    return sorted(found)


def names_for(root: Path, constant: Constant) -> dict[str, int]:
    """Значения соседних числовых пределов: сначала того же файла, затем крейта."""
    values: dict[str, int] = {}
    parsed = sources(root)
    crate = parsed[constant.file].crate
    for path, source in sorted(parsed.items(), key=lambda item: item[0] == constant.file):
        if source.crate != crate:
            continue
        for item in source.constants:
            value = evaluate(item.expr, "count")
            if value is not None:
                values[item.symbol] = value
    return values


def load(root: Path) -> dict:
    return json.loads((root / REGISTRY).read_text(encoding="utf-8"))


def schema_errors(root: Path, registry: dict) -> list[str]:
    import jsonschema

    schema = json.loads((root / SCHEMA).read_text(encoding="utf-8"))
    validator = jsonschema.Draft202012Validator(schema)
    return [
        f"{REGISTRY}: {'/'.join(map(str, error.absolute_path)) or '<root>'}: {error.message}"
        for error in sorted(validator.iter_errors(registry), key=lambda item: list(map(str, item.absolute_path)))
    ]


def check(root: Path) -> list[str]:
    sources.cache_clear()
    registry = load(root)
    errors = schema_errors(root, registry)
    if errors:
        return errors
    constants: dict[tuple, Constant] = {}
    for constant in scan(root):
        item_key = key(constant.file, constant.symbol, constant.within)
        if item_key in constants:
            errors.append(f"{constant.file}:{constant.line}: {constant.symbol} is declared twice in one scope; the registry cannot tell them apart")
        constants[item_key] = constant
    seen: dict[tuple, str] = {}
    ids: set[str] = set()
    for entry in registry["limits"]:
        ident = entry["id"]
        if ident in ids:
            errors.append(f"{ident}: duplicate id")
        ids.add(ident)
        source = entry["source"]
        file = source["file"]
        if not (root / file).is_file():
            errors.append(f"{ident}: source file {file} does not exist")
            continue
        if "literal" in source:
            count = literal_count(root, file, source["literal"])
            if count != source["count"]:
                errors.append(f"{ident}: literal {source['literal']!r} occurs {count} times in {file}, registry says {source['count']}")
            if normalize(entry["expr"]) not in normalize(source["literal"]):
                errors.append(f"{ident}: expr `{entry['expr']}` is not part of literal {source['literal']!r}")
            computed = evaluate(entry["expr"], entry["unit"])
            if computed != entry["value"]:
                errors.append(f"{ident}: `{entry['expr']}` evaluates to {computed}, registry value is {entry['value']}")
            if entry["applied"] != [file]:
                errors.append(f"{ident}: a literal applies only in {file}")
            continue
        item_key = key(file, source["symbol"], source.get("within"))
        if item_key in seen:
            errors.append(f"{ident}: {file}::{source['symbol']} is already registered as {seen[item_key]}")
        seen[item_key] = ident
        constant = constants.get(item_key)
        if constant is None:
            errors.append(f"{ident}: {file} has no product limit constant {source['symbol']} within {source.get('within')}")
            continue
        if normalize(constant.expr) != normalize(entry["expr"]):
            errors.append(f"{ident}: code has `{constant.expr}`, registry has `{entry['expr']}`")
        plain = evaluate(constant.expr, entry["unit"])
        computed = plain if plain is not None else evaluate(constant.expr, entry["unit"], names_for(root, constant))
        if computed is not None and computed != entry["value"]:
            errors.append(f"{ident}: `{constant.expr}` evaluates to {computed}, registry value is {entry['value']}")
        if plain is None and not entry.get("derived"):
            errors.append(f"{ident}: `{constant.expr}` is not a plain number; mark the entry as derived")
        if plain is not None and entry.get("derived"):
            errors.append(f"{ident}: `{constant.expr}` is a plain number; derived is not needed")
        applied = users(root, constant)
        if sorted(entry["applied"]) != applied:
            errors.append(f"{ident}: applied files {sorted(entry['applied'])} differ from product users {applied}")
    for item in registry["excluded"]:
        item_key = key(item["file"], item["symbol"], item.get("within"))
        if item_key in seen:
            errors.append(f"{item['file']}::{item['symbol']} is both registered and excluded")
        elif item_key not in constants:
            errors.append(f"excluded {item['file']}::{item['symbol']} no longer exists")
        seen[item_key] = "excluded"
    for item_key, constant in sorted(constants.items(), key=lambda pair: (pair[1].file, pair[1].line)):
        if item_key not in seen:
            errors.append(f"{constant.file}:{constant.line}: {constant.symbol} is not in {REGISTRY}; run scripts/ci/check-resource-limits.py --scaffold")
    return errors


def scaffold(root: Path) -> list[dict]:
    sources.cache_clear()
    registry = load(root)
    known = {key(entry["source"]["file"], entry["source"].get("symbol"), entry["source"].get("within")) for entry in registry["limits"]}
    known |= {key(item["file"], item["symbol"], item.get("within")) for item in registry["excluded"]}
    entries = []
    for constant in scan(root):
        if key(constant.file, constant.symbol, constant.within) in known:
            continue
        unit = "ms" if "Duration" in constant.type or "Duration" in constant.expr else "count"
        source = {"file": constant.file, "symbol": constant.symbol}
        if constant.within:
            source["within"] = constant.within
        plain = evaluate(constant.expr, unit)
        entry = {
            "id": constant.symbol.lower().replace("_", "-"),
            "class": None,
            "resource": None,
            "unit": unit,
            "scope": None,
            "source": source,
            "expr": constant.expr,
            "value": plain if plain is not None else evaluate(constant.expr, unit, names_for(root, constant)),
            "applied": users(root, constant),
            "status": "unproven",
            "origin": None,
            "problem": None,
            "onReach": None,
            "issue": None,
            "evidence": [],
            "decision": None,
            "review": None,
        }
        if plain is None:
            entry["derived"] = True
        entries.append(entry)
    return entries


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--scaffold", action="store_true", help="print draft entries for unregistered constants")
    args = parser.parse_args()
    root = args.root.resolve()
    if args.scaffold:
        print(json.dumps(scaffold(root), ensure_ascii=False, indent=2))
        return 0
    errors = check(root)
    for error in errors:
        print(error, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
