#!/usr/bin/env python3
"""Сверка ресурсных ограничений продукта с их описанием в коде.

Число предела живёт в коде, а сведения о нём — в блоке комментариев прямо
над константой или над строкой, где предел записан литералом:

    // resource-limit: lsp.completion-inference-depth
    // class: step
    // unit: depth
    // scope: request
    // value: 4
    // resource: Шаги вывода типа при дополнении.
    // status: unproven
    // on-reach: тип дальше не прослеживается
    pub const MAX_INFERENCE_DEPTH: usize = 4;

Константу, имя которой похоже на предел (`LIMIT_NAME`), но которая им не
является, помечает `// resource-limit-excluded: <причина>`. Ошибка — значение,
расходящееся с выражением под блоком; константа-предел без блока; блок, под
которым нет предела; повтор id. Предел без такого имени, в том числе
литерал, страж сам не находит: блок ставится вручную и затем сверяется.

`--report` печатает опись пределов JSON, `--scaffold` — заготовки блоков для
констант без описания.
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

# Имя константы похоже на ресурсный предел, срок, окно, порцию или ёмкость.
LIMIT_NAME = re.compile(
    r"(^MAX_|_MAX_|_MAX$|LIMIT|BUDGET|_CAP$|_CAP_|QUOTA|TIMEOUT|DEADLINE|_TTL|^TTL_|TTL$"
    r"|THRESHOLD|_WINDOW$|CAPACITY|^RETAINED_|_BYTES$|_SECONDS$|_SECS$|_MS$|PAGE_SIZE|PAGE_BYTES|CHUNK"
    r"|RETRIES|RETRY|ATTEMPTS|GRACE|RETENTION|_INTERVAL$|_DELAY$|_DEPTH$|BOUND|_POLL$|STACK_SIZE)"
)
DURATION = re.compile(r"^(?:std::time::)?Duration::from_(secs|millis|mins)\((.+)\)$", re.S)
UNIT_MS = {"secs": 1000, "millis": 1, "mins": 60_000}
ITEM_KINDS = {"const_item", "static_item"}

MARK = "resource-limit"
EXCLUDED = "resource-limit-excluded"
LINE = re.compile(r"^[ \t]*// ([a-z][a-z-]*): (.+?)\s*$")

CLASSES = {"corpus", "step", "retention", "concurrency", "retry", "format", "deadline", "result"}
UNITS = {"bytes", "count", "depth", "ms", "s", "chars"}
SCOPES = {
    "request", "file", "line", "directory", "traversal", "source-set", "workspace", "repository",
    "record", "frame", "process", "daemon", "connection", "session", "cache", "operation",
}
STATUSES = {"unproven", "justified", "external"}
ID = re.compile(r"^[a-z0-9]+(?:[.-][a-z0-9]+)*$")
ISSUE = re.compile(r"^#[0-9]+$")
DATE = re.compile(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$")
# Поля блока в том порядке, в каком они пишутся; `evidence` повторяется.
FIELDS = (
    "class", "unit", "scope", "value", "derived", "literal", "count", "expr", "resource", "status",
    "origin", "problem", "on-reach", "issue", "evidence", "decision", "decision-date",
    "decision-source", "review",
)
REQUIRED = ("class", "unit", "scope", "resource", "status", "on-reach")


class Item(NamedTuple):
    """Константа или статическое значение продукта."""

    file: str
    line: int
    symbol: str
    type: str
    expr: str


class Block(NamedTuple):
    """Описание предела или исключения в коде."""

    file: str
    line: int
    end: int
    kind: str
    head: str
    fields: tuple[tuple[str, str], ...]


class Source(NamedTuple):
    crate: str
    text: str
    items: tuple[Item, ...]
    blocks: tuple[Block, ...]


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


def read_blocks(rel: str, lines: list[str]) -> list[Block]:
    """Блоки `// resource-limit…` текста: заголовок и следующие строки `// поле: значение`."""
    blocks = []
    index = 0
    while index < len(lines):
        match = LINE.match(lines[index])
        if not match or match.group(1) not in (MARK, EXCLUDED):
            index += 1
            continue
        start = index
        fields = []
        index += 1
        while index < len(lines):
            field = LINE.match(lines[index])
            if not field or field.group(1) in (MARK, EXCLUDED):
                break
            fields.append((field.group(1), field.group(2)))
            index += 1
        blocks.append(Block(rel, start + 1, index, match.group(1), match.group(2), tuple(fields)))
    return blocks


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
    items = []

    def walk(node):
        if (node.start_byte, node.end_byte) in skipped:
            return
        if node.type in ITEM_KINDS:
            value = node.child_by_field_name("value")
            kind = node.child_by_field_name("type")
            if value is not None and kind is not None:
                name = node.child_by_field_name("name").text.decode()
                items.append(Item(rel, node.start_point[0] + 1, name, kind.text.decode(), value.text.decode().strip()))
        for child in node.children:
            walk(child)

    walk(tree.root_node)
    lines = text.splitlines(keepends=True)
    blocks = read_blocks(rel, lines)
    # A block names its literal: only the code counts as its occurrences.
    for block in blocks:
        for index in range(block.line - 1, block.end):
            lines[index] = "\n"
    return Source(rel.split("/")[1], "".join(lines), tuple(items), tuple(blocks))


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


def is_limit(item: Item) -> bool:
    """Константа с именем предела; `pub const X: T = super::X;` переэкспортирует уже описанный."""
    return bool(LIMIT_NAME.search(item.symbol)) and not normalize(item.expr).endswith(f"::{item.symbol}")


def evaluate(expr: str, unit: str, names: dict[str, int] | None = None) -> int | None:
    """Значение выражения в единице предела; `None`, если выражение не вычисляется."""
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


def names_for(root: Path, item: Item) -> dict[str, int]:
    """Значения соседних числовых пределов: сначала того же файла, затем крейта."""
    values: dict[str, int] = {}
    parsed = sources(root)
    crate = parsed[item.file].crate
    for path, source in sorted(parsed.items(), key=lambda pair: pair[0] == item.file):
        if source.crate != crate:
            continue
        for other in source.items:
            if not is_limit(other):
                continue
            value = evaluate(other.expr, "count")
            if value is not None:
                values[other.symbol] = value
    return values


def literal_position(text: str, after: int, literal: str) -> tuple[int, int] | None:
    """Строка первого вхождения литерала после строки `after` и число его вхождений в файле."""
    lines = text.splitlines(keepends=True)
    target = normalize(literal)
    tail = normalize("".join(lines[after:]))
    found = tail.find(target)
    if found < 0:
        return None
    consumed = 0
    for offset, line in enumerate(lines[after:]):
        consumed += len(normalize(line))
        if consumed > found:
            return after + offset + 1, normalize(text).count(target)
    return None


def field_errors(where: str, block: Block) -> tuple[dict, list[str]]:
    """Поля блока описью предела и ошибки их формы."""
    errors = []
    entry: dict = {"id": block.head}
    evidence = []
    seen = set()
    order = []
    for name, value in block.fields:
        if name not in FIELDS:
            errors.append(f"{where}: unknown field `{name}`")
            continue
        if name == "evidence":
            evidence.append(value)
        elif name in seen:
            errors.append(f"{where}: field `{name}` repeats")
        else:
            entry[name] = value
        seen.add(name)
        order.append(FIELDS.index(name))
    if order != sorted(order):
        errors.append(f"{where}: fields go in the order {', '.join(FIELDS)}")
    if not ID.match(block.head):
        errors.append(f"{where}: id `{block.head}` is not lower-case words joined by `.` or `-`")
    for name in REQUIRED:
        if name not in entry:
            errors.append(f"{where}: field `{name}` is missing")
    for name, allowed in (("class", CLASSES), ("unit", UNITS), ("scope", SCOPES), ("status", STATUSES)):
        if name in entry and entry[name] not in allowed:
            errors.append(f"{where}: {name} `{entry[name]}` is not one of {', '.join(sorted(allowed))}")
    if "issue" in entry and not ISSUE.match(entry["issue"]):
        errors.append(f"{where}: issue `{entry['issue']}` is not `#<number>`")
    if "derived" in entry and entry["derived"] != "true":
        errors.append(f"{where}: derived is written only as `derived: true`")
    for name in ("value", "count"):
        if name in entry and not entry[name].isdigit():
            errors.append(f"{where}: {name} `{entry[name]}` is not a number")
    decision = [name for name in ("decision", "decision-date", "decision-source") if name in entry]
    if decision and len(decision) != 3:
        errors.append(f"{where}: decision, decision-date and decision-source go together")
    if "decision-date" in entry and not DATE.match(entry["decision-date"]):
        errors.append(f"{where}: decision-date `{entry['decision-date']}` is not YYYY-MM-DD")
    if entry.get("status") == "justified" and not (evidence and len(decision) == 3):
        errors.append(f"{where}: a justified limit names its evidence and decision")
    if entry.get("status") == "external" and "origin" not in entry:
        errors.append(f"{where}: an external limit names its origin")
    if "literal" in entry and ("count" not in entry or "expr" not in entry):
        errors.append(f"{where}: a literal limit names its count and expr")
    if "literal" not in entry and ("count" in entry or "expr" in entry):
        errors.append(f"{where}: count and expr belong to a literal limit; a constant gives its own expr")
    record = {
        "id": block.head,
        "class": entry.get("class"),
        "resource": entry.get("resource"),
        "unit": entry.get("unit"),
        "scope": entry.get("scope"),
        "expr": entry.get("expr"),
        "value": int(entry["value"]) if entry.get("value", "").isdigit() else None,
        "status": entry.get("status"),
        "origin": entry.get("origin"),
        "problem": entry.get("problem"),
        "onReach": entry.get("on-reach"),
        "issue": entry.get("issue"),
        "evidence": evidence,
        "decision": (
            {"date": entry["decision-date"], "text": entry["decision"], "source": entry["decision-source"]}
            if len(decision) == 3 else None
        ),
        "review": entry.get("review"),
    }
    if entry.get("derived") == "true":
        record["derived"] = True
    if "literal" in entry:
        record["literal"] = entry["literal"]
        record["count"] = int(entry["count"]) if entry.get("count", "").isdigit() else None
    return record, errors


def inventory(root: Path) -> tuple[dict, list[str]]:
    """Опись пределов из кода и ошибки сверки."""
    sources.cache_clear()
    errors: list[str] = []
    limits: list[dict] = []
    excluded: list[dict] = []
    ids: dict[str, str] = {}
    described: set[tuple[str, int]] = set()
    for file, source in sources(root).items():
        by_line = {item.line: item for item in source.items}
        for block in source.blocks:
            where = f"{file}:{block.line}"
            target = by_line.get(block.end + 1)
            if block.kind == EXCLUDED:
                if block.fields:
                    errors.append(f"{where}: an exclusion has only its reason")
                if target is None:
                    errors.append(f"{where}: resource-limit-excluded stands before no constant")
                    continue
                described.add((file, target.line))
                excluded.append({"at": f"{file}:{target.line}", "symbol": target.symbol, "reason": block.head})
                continue
            record, problems = field_errors(where, block)
            errors.extend(problems)
            if block.head in ids:
                errors.append(f"{where}: id {block.head} is already used at {ids[block.head]}")
            ids[block.head] = where
            unit = record["unit"] or "count"
            if "literal" in record:
                position = literal_position(source.text, block.end, record["literal"])
                if position is None or position[0] != block.end + 1:
                    errors.append(f"{where}: {block.head} stands before no line with literal {record['literal']!r}")
                elif record["count"] is not None and position[1] != record["count"]:
                    errors.append(
                        f"{where}: {block.head}: literal {record['literal']!r} occurs {position[1]} times in {file}, "
                        f"the block says {record['count']}"
                    )
                if record["expr"] is not None:
                    if normalize(record["expr"]) not in normalize(record["literal"]):
                        errors.append(f"{where}: {block.head}: expr `{record['expr']}` is not part of literal {record['literal']!r}")
                    computed = evaluate(record["expr"], unit)
                    if computed != record["value"]:
                        errors.append(f"{where}: {block.head}: `{record['expr']}` evaluates to {computed}, the block says {record['value']}")
                record["at"] = f"{file}:{block.end + 1}"
                limits.append(record)
                continue
            if target is None:
                errors.append(f"{where}: {block.head} stands before no constant; put the block right above it")
                continue
            described.add((file, target.line))
            record["expr"] = target.expr
            plain = evaluate(target.expr, unit)
            computed = plain if plain is not None else evaluate(target.expr, unit, names_for(root, target))
            if computed is not None and computed != record["value"]:
                errors.append(f"{where}: {block.head}: `{target.expr}` evaluates to {computed}, the block says {record['value']}")
            if plain is None and not record.get("derived"):
                errors.append(f"{where}: {block.head}: `{target.expr}` is not a plain number; mark it `derived: true`")
            if plain is not None and record.get("derived"):
                errors.append(f"{where}: {block.head}: `{target.expr}` is a plain number; derived is not needed")
            record["at"] = f"{file}:{target.line}"
            limits.append(record)
        for item in source.items:
            if is_limit(item) and (file, item.line) not in described:
                errors.append(
                    f"{file}:{item.line}: {item.symbol} has no `// {MARK}:` block above it; "
                    "run scripts/ci/check-resource-limits.py --scaffold"
                )
    return {"limits": limits, "excluded": excluded}, errors


def check(root: Path) -> list[str]:
    return inventory(root)[1]


def scaffold(root: Path) -> list[str]:
    """Заготовки блоков для констант-пределов без описания."""
    _, errors = inventory(root)
    missing = {error.split(": ", 1)[0] for error in errors if f"no `// {MARK}:` block" in error}
    drafts = []
    for file, source in sources(root).items():
        for item in source.items:
            if f"{file}:{item.line}" not in missing:
                continue
            unit = "ms" if "Duration" in item.type or "Duration" in item.expr else "count"
            plain = evaluate(item.expr, unit)
            value = plain if plain is not None else evaluate(item.expr, unit, names_for(root, item))
            lines = [
                f"{file}:{item.line} {item.symbol}",
                f"// {MARK}: {item.symbol.lower().replace('_', '-')}",
                "// class: <corpus|step|retention|concurrency|retry|format|deadline|result>",
                f"// unit: {unit}",
                "// scope: <request|file|...>",
            ]
            if value is not None:
                lines.append(f"// value: {value}")
            if plain is None:
                lines.append("// derived: true")
            lines += ["// resource: <что ограничено>", "// status: unproven", "// on-reach: <что происходит у предела>"]
            drafts.append("\n".join(lines))
    return drafts


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, default=ROOT)
    action = parser.add_mutually_exclusive_group()
    action.add_argument("--report", action="store_true", help="print the inventory of limits as JSON")
    action.add_argument("--scaffold", action="store_true", help="print draft blocks for undescribed constants")
    args = parser.parse_args()
    root = args.root.resolve()
    if args.scaffold:
        print("\n\n".join(scaffold(root)))
        return 0
    found, errors = inventory(root)
    if args.report:
        print(json.dumps(found, ensure_ascii=False, indent=2))
    for error in errors:
        print(error, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
