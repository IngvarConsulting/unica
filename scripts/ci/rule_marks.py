"""Пометки `rule: <ID>` на тестах: чтение, перенос из `check` и сверка пар.

Тест сам называет правило, которое подтверждает. В Rust пометка — строка
комментария `/// rule: ID` (или `// rule: ID`) в блоке комментариев и атрибутов
прямо над тестом, над вызовом макроса, порождающего тест, или над `mod`: тогда
она относится ко всем тестам модуля. Внутренний `//! rule: ID` относится ко всем
тестам файла. В Python пометка — строка `rule: ID` в docstring метода или класса.

Адрес теста — `путь.rs::[модуль::]имя`, где модули — встроенные `mod` этого
файла, и `путь.py::Класс.метод`. Те же адреса раньше перечислялись в `check`.
"""

from __future__ import annotations

import ast
import json
import re
import tomllib
import warnings
from dataclasses import dataclass
from pathlib import Path

RULES = Path("arch/rules")
FRONT_MATTER = re.compile(r"\A---\n(.*?)\n---\n?", re.S)
RULE_ID = r"[A-Z][A-Z0-9]*(?:\.[A-Z0-9][A-Z0-9-]*)+"
RUST_MARK = re.compile(rf"^\s*//[/!]?\s*rule:\s*({RULE_ID})\s*$")
PYTHON_MARK = re.compile(rf"^\s*rule:\s*({RULE_ID})\s*$", re.M)
TEST_ATTRIBUTE = re.compile(r"#\[\s*(?:\w+::)*test\b")


@dataclass(frozen=True)
class RustItem:
    """Функция или вызов макроса в файле с цепочкой встроенных модулей."""

    kind: str  # "fn" | "macro" | "mod"
    name: str
    modules: tuple[str, ...]
    line: int  # индекс строки начала элемента (после атрибутов)
    head: int  # индекс первой строки блока комментариев и атрибутов
    is_test: bool
    marks: tuple[str, ...]
    target: str | None = None  # `#[path]` модуля-файла или путь `include!`
    mark_lines: tuple[int, ...] = ()


def rule_ids(root: Path) -> set[str]:
    ids = set()
    for path in (root / RULES).rglob("*.md"):
        match = FRONT_MATTER.match(path.read_text(encoding="utf-8").replace("\r\n", "\n"))
        if match:
            found = re.search(r"^id:\s*(\S+)\s*$", match.group(1), re.M)
            if found:
                ids.add(found.group(1))
    return ids


# --- Rust ------------------------------------------------------------------


_LEXEME = re.compile(
    r"//[^\n]*"
    r"|/\*.*?\*/"
    r'|(?<!\w)b?r(?P<hashes>#*)".*?"(?P=hashes)'
    r'|(?<!\w)b?"(?:\\.|[^"\\])*"'
    r"|'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'\n])'",
    re.S,
)
_NOT_NEWLINE = re.compile(r"[^\n]")


def _blank_code(source: str) -> str:
    """Заменить строки, символы и комментарии пробелами, сохранив переводы строк."""

    return _LEXEME.sub(lambda match: _NOT_NEWLINE.sub(" ", match.group(0)), source)


def rust_items(source: str) -> list[RustItem]:
    """Тесты, вызовы макросов и модули файла с их пометками."""

    raw_lines = source.splitlines()
    code = _blank_code(source)
    code_lines = code.splitlines()
    items: list[RustItem] = []
    stack: list[tuple[str | None, int]] = []  # (имя модуля или None, глубина скобок)
    depth = 0
    pending_mod: str | None = None
    # Позиции начала строк для перевода смещения в номер строки.
    starts = [0]
    for line in code.splitlines(keepends=True):
        starts.append(starts[-1] + len(line))

    def line_of(offset: int) -> int:
        lo, hi = 0, len(starts) - 1
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if starts[mid] <= offset:
                lo = mid
            else:
                hi = mid - 1
        return lo

    def header(index: int) -> tuple[int, bool, tuple[str, ...]]:
        """Подняться над элементом по атрибутам и комментариям."""

        cursor = index - 1
        is_test = False
        marks: list[str] = []
        lines_of_marks.clear()
        while cursor >= 0:
            text = raw_lines[cursor].strip()
            if not text:
                break
            if text.startswith("//"):
                if m := RUST_MARK.match(raw_lines[cursor]):
                    if not text.startswith("//!"):
                        marks.append(m.group(1))
                        lines_of_marks.append(cursor)
                cursor -= 1
                continue
            if text.startswith("#[") or text.endswith("]") or text.startswith("#!"):
                if TEST_ATTRIBUTE.search(text):
                    is_test = True
                cursor -= 1
                continue
            break
        # Атрибут на той же строке, что и fn: `#[test] fn x()`.
        if TEST_ATTRIBUTE.search(raw_lines[index].split("fn ")[0]):
            is_test = True
        return cursor + 1, is_test, tuple(reversed(marks))

    lines_of_marks: list[int] = []
    token = re.compile(r"\{|\}|\bmod\s+(\w+)\s*(\{|;)|\bfn\s+(\w+)|\b(\w+)!\s*[(\[{]")
    for m in token.finditer(code):
        text = m.group(0)
        if text == "{":
            depth += 1
            stack.append((pending_mod, depth))
            pending_mod = None
            continue
        if text == "}":
            if stack and stack[-1][1] == depth:
                stack.pop()
            depth -= 1
            continue
        modules = tuple(name for name, _ in stack if name)
        line = line_of(m.start())
        if m.group(1):
            head, _, marks = header(line)
            if m.group(2) == ";":
                attribute = next(
                    (
                        found.group(1)
                        for text_line in [*raw_lines[head:line], raw_lines[line].split("mod ")[0]]
                        if (found := re.search(r'#\[\s*path\s*=\s*"([^"]+)"', text_line))
                    ),
                    None,
                )
                items.append(RustItem("modfile", m.group(1), modules, line, head, False, marks, attribute, tuple(lines_of_marks)))
                continue
            items.append(RustItem("mod", m.group(1), modules, line, head, False, marks, None, tuple(lines_of_marks)))
            depth += 1
            stack.append((m.group(1), depth))
            continue
        if m.group(3):
            head, is_test, marks = header(line)
            items.append(RustItem("fn", m.group(3), modules, line, head, is_test, marks, None, tuple(lines_of_marks)))
            continue
        if m.group(4) == "include":
            found = re.match(r'\s*(concat!\s*\(\s*env!\s*\(\s*"CARGO_MANIFEST_DIR"\s*\)\s*,\s*)?"([^"]+)"', source[m.end():m.end() + 300])
            if found:
                # Путь от каталога пакета отмечен префиксом `@crate`.
                target = ("@crate" if found.group(1) else "") + found.group(2)
                items.append(RustItem("include", found.group(2), modules, line, line, False, (), target))
        elif m.group(4) and m.group(4) not in {"macro_rules", "include", "include_str", "concat", "format", "vec", "assert", "assert_eq", "assert_ne", "println", "panic", "write", "writeln", "json", "matches", "debug_assert"}:
            # Имя теста — первый аргумент вызова макроса.
            rest = source[m.end():m.end() + 400]
            rest = re.sub(r"^\s*(?:#\[[^\]]*\]\s*)*", "", rest)
            first = re.match(r"(\w+)", rest)
            if first:
                head, _, marks = header(line)
                items.append(RustItem("macro", first.group(1), modules, line, head, False, marks, None, tuple(lines_of_marks)))
        # открывающая скобка вызова учитывается следующим совпадением `{`
        if text.endswith("{"):
            depth += 1
            stack.append((None, depth))
    return items


def macro_generates_tests(source: str, macro: str) -> bool:
    definition = re.search(rf"macro_rules!\s*{macro}\s*\{{", source)
    return bool(definition and TEST_ATTRIBUTE.search(source[definition.end():definition.end() + 4000]))


def file_marks(source: str) -> tuple[str, ...]:
    marks = []
    for line in source.splitlines():
        text = line.strip()
        if text.startswith("//!"):
            if m := RUST_MARK.match(line):
                marks.append(m.group(1))
        elif text and not text.startswith("//") and not text.startswith("#!"):
            break
    return tuple(marks)




def rust_mark_usage(source: str) -> tuple[set[tuple[str, tuple[str, ...], str]], set[int], list[int]]:
    """Тесты с правилами, использованные строки пометок и все строки пометок файла."""

    items = rust_items(source)
    lines = source.splitlines()
    all_lines = [index for index, line in enumerate(lines) if RUST_MARK.match(line)]
    tests = []
    for item in items:
        if (item.kind == "fn" and item.is_test) or (
            item.kind == "macro" and (name := _macro_name(source, item)) and macro_generates_tests(source, name)
        ):
            tests.append(item)
    used: set[int] = set()
    found: set[tuple[str, tuple[str, ...], str]] = set()
    file_lines = []
    for index, line in enumerate(lines):
        text = line.strip()
        if text.startswith("//!"):
            if RUST_MARK.match(line):
                file_lines.append(index)
        elif text and not text.startswith("//") and not text.startswith("#!"):
            break
    module_items = [item for item in items if item.kind == "mod" and item.marks]
    for test in tests:
        marks = set(test.marks)
        used |= set(test.mark_lines)
        if file_lines:
            marks |= {RUST_MARK.match(lines[index]).group(1) for index in file_lines}
            used |= set(file_lines)
        for module in module_items:
            path = module.modules + (module.name,)
            if test.modules[: len(path)] == path:
                marks |= set(module.marks)
                used |= set(module.mark_lines)
        found |= {(rule, test.modules, test.name) for rule in marks}
    return found, used, all_lines


def rust_marked_pairs(path: str, source: str) -> set[tuple[str, str]]:
    """Пары (правило, адрес) из пометок одного Rust-файла."""

    found, _, _ = rust_mark_usage(source)
    return {(rule, "::".join((path, *modules, name))) for rule, modules, name in found}


def _macro_name(source: str, item: RustItem) -> str | None:
    text = source.splitlines()[item.line]
    m = re.search(r"\b(\w+)!\s*[(\[{]", text)
    return m.group(1) if m else None


# --- Python ----------------------------------------------------------------


def _parse(source: str) -> ast.AST:
    """Разобрать исходник, не печатая предупреждений чужого кода."""

    with warnings.catch_warnings():
        warnings.simplefilter("ignore", SyntaxWarning)
        return ast.parse(source)


def python_marked_pairs(path: str, source: str) -> set[tuple[str, str]]:
    pairs = set()
    tree = _parse(source)
    for node in ast.walk(tree):
        if not isinstance(node, ast.ClassDef):
            continue
        class_marks = set(PYTHON_MARK.findall(ast.get_docstring(node) or ""))
        for item in node.body:
            if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name.startswith("test"):
                marks = class_marks | set(PYTHON_MARK.findall(ast.get_docstring(item) or ""))
                pairs |= {(rule, f"{path}::{node.name}.{item.name}") for rule in marks}
    return pairs


# --- Сбор ------------------------------------------------------------------


def source_files(root: Path) -> list[Path]:
    files = [p for p in (root / "crates").rglob("*.rs") if "target" not in p.parts]
    files += [p for p in root.glob("tests/**/*.py")]
    files += [p for p in root.glob("scripts/**/*.py")]
    return sorted(p for p in files if "fixtures" not in p.relative_to(root).parts)


def marked_pairs(root: Path) -> set[tuple[str, str]]:
    pairs: set[tuple[str, str]] = set()
    for path in source_files(root):
        text = path.read_text(encoding="utf-8", errors="replace")
        if "rule:" not in text:
            continue
        relative = path.relative_to(root).as_posix()
        if path.suffix == ".rs":
            pairs |= rust_marked_pairs(relative, text)
        else:
            pairs |= python_marked_pairs(relative, text)
    return pairs


def python_mark_usage(source: str) -> tuple[list[tuple[str, int]], list[tuple[str, int]]]:
    """Пометки Python: (правило, строка) на тестах и вне тестов."""

    used, stray = [], []
    tree = _parse(source)
    owners: dict[ast.AST, bool] = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.ClassDef):
            tests = [
                item
                for item in node.body
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name.startswith("test")
            ]
            owners[node] = bool(tests)
            for item in tests:
                owners[item] = True
    for node in ast.walk(tree):
        if not isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        doc = ast.get_docstring(node)
        if not doc:
            continue
        line = node.body[0].lineno if node.body else 0
        target = used if owners.get(node, False) else stray
        target.extend((rule, line) for rule in PYTHON_MARK.findall(doc))
    return used, stray


def stray_marks(root: Path) -> list[str]:
    """Пометки с неизвестным правилом и пометки, не относящиеся к тесту."""

    ids = rule_ids(root)
    problems = []
    for path in source_files(root):
        text = path.read_text(encoding="utf-8", errors="replace")
        if "rule:" not in text:
            continue
        relative = path.relative_to(root).as_posix()
        if path.suffix == ".rs":
            _, used_lines, all_lines = rust_mark_usage(text)
            lines = text.splitlines()
            marks = [(RUST_MARK.match(lines[index]).group(1), index + 1, index in used_lines) for index in all_lines]
        else:
            used, stray = python_mark_usage(text)
            marks = [(rule, line, True) for rule, line in used] + [(rule, line, False) for rule, line in stray]
        for rule, line, on_test in sorted(marks, key=lambda mark: (mark[1], mark[0])):
            if rule not in ids:
                problems.append(f"{relative}:{line}: rule {rule} is not defined in arch/rules")
            elif not on_test:
                problems.append(f"{relative}:{line}: rule {rule} is marked, but not on a test")
    return problems


def rust_test_items(source: str) -> list[RustItem]:
    out = []
    for item in rust_items(source):
        if item.kind == "fn" and item.is_test:
            out.append(item)
        elif item.kind == "macro":
            name = _macro_name(source, item)
            if name and macro_generates_tests(source, name):
                out.append(item)
    return out


# --- Перенос из `check` ----------------------------------------------------

RUST_ADDRESS = re.compile(r"^(\S+\.rs)::((?:\w+::)*)(\w+)$")
PYTHON_ADDRESS = re.compile(r"^(\S+\.py)::(\w+)\.(\w+)$")


def check_addresses(text: str) -> list[str]:
    match = FRONT_MATTER.match(text.replace("\r\n", "\n"))
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


def check_pairs(root: Path) -> set[tuple[str, str]]:
    pairs = set()
    for path in sorted((root / RULES).rglob("*.md")):
        text = path.read_text(encoding="utf-8")
        found = re.search(r"^id:\s*(\S+)\s*$", text, re.M)
        if found:
            pairs |= {(found.group(1), address) for address in check_addresses(text)}
    return pairs


def strip_check(text: str) -> str:
    """Удалить поле `check` из заголовка правила, не трогая остальной текст."""

    newline = "\r\n" if "\r\n" in text else "\n"
    body = text.replace("\r\n", "\n")
    match = FRONT_MATTER.match(body)
    if match is None:
        return text
    kept, skipping = [], False
    for line in match.group(1).split("\n"):
        if re.match(r"^\S", line):
            skipping = line.startswith("check:")
        if not skipping:
            kept.append(line)
    if len(kept) == len(match.group(1).split("\n")):
        return text
    head = "---\n" + "\n".join(kept) + "\n---\n"
    return (head + body[match.end():]).replace("\n", newline)


def resolve_rust(tests: list[RustItem], modules: tuple[str, ...], name: str) -> RustItem | None:
    candidates = [item for item in tests if item.name == name]
    exact = [item for item in candidates if item.modules == modules]
    if len(exact) == 1:
        return exact[0]
    if modules:
        suffix = [item for item in candidates if item.modules[-len(modules):] == modules]
        if len(suffix) == 1:
            return suffix[0]
    return candidates[0] if len(candidates) == 1 else None


def _python_method(tree: ast.AST, cls: str, method: str):
    for top in ast.walk(tree):
        if isinstance(top, ast.ClassDef) and top.name == cls:
            for item in top.body:
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name == method:
                    return item
    return None


def _mark_python(lines: list[str], node, rules: list[str]) -> None:
    """Дописать строки `rule: ID` в docstring метода или создать его."""

    first = node.body[0]
    start = first.lineno - 1
    indent = re.match(r"\s*", lines[start]).group(0)
    marks = [f"{indent}rule: {rule}" for rule in rules]
    is_doc = (
        isinstance(first, ast.Expr)
        and isinstance(first.value, ast.Constant)
        and isinstance(first.value.value, str)
    )
    if not is_doc:
        if len(rules) == 1:
            lines[start:start] = [f'{indent}"""rule: {rules[0]}"""']
        else:
            lines[start:start] = [f'{indent}"""', *marks, f'{indent}"""']
        return
    end = first.end_lineno - 1
    closing = lines[end].rstrip()
    quote = closing[-3:]
    if quote not in ('"""', "'''"):
        raise ValueError(f"line {end + 1}: unsupported docstring form")
    content = closing[:-3]
    if start == end:
        lines[end:end + 1] = [content, "", *marks, f"{indent}{quote}"]
    elif content.strip():
        lines[end:end + 1] = [content, "", *marks, f"{indent}{quote}"]
    else:
        lines[end:end] = ["", *marks]


def migrate(root: Path) -> tuple[set[tuple[str, str]], set[tuple[str, str]], list[str]]:
    """Перенести `check` в пометки; вернуть пары до (канонические), после и проблемы."""

    by_file: dict[str, list[tuple[str, str]]] = {}
    problems: list[str] = []
    for rule, address in sorted(check_pairs(root)):
        match = RUST_ADDRESS.match(address) or PYTHON_ADDRESS.match(address)
        if match is None:
            problems.append(f"{rule}: {address}: unrecognized address form")
            continue
        by_file.setdefault(match.group(1), []).append((rule, address))
    before: set[tuple[str, str]] = set()
    renamed: list[str] = []
    for relative, wanted in sorted(by_file.items()):
        path = root / relative
        if not path.is_file():
            problems.extend(f"{rule}: {address}: no such file" for rule, address in wanted)
            continue
        raw = path.read_bytes().decode("utf-8")
        newline = "\r\n" if "\r\n" in raw else "\n"
        source = raw.replace("\r\n", "\n")
        lines = source.split("\n")
        if relative.endswith(".rs"):
            inserts: dict[int, list[str]] = {}
            tests = rust_test_items(source)
            for rule, address in wanted:
                _, modules, name = RUST_ADDRESS.match(address).groups()
                item = resolve_rust(tests, tuple(m for m in modules.split("::") if m), name)
                if item is None:
                    problems.append(f"{rule}: {address}: test is not found or is ambiguous")
                    continue
                canonical = "::".join((relative, *item.modules, item.name))
                if canonical != address:
                    renamed.append(f"{address} -> {canonical}")
                before.add((rule, canonical))
                if rule not in item.marks and rule not in inserts.get(item.head, []):
                    inserts.setdefault(item.head, []).append(rule)
            for head in sorted(inserts, reverse=True):
                cursor = head
                while cursor < len(lines) and lines[cursor].strip().startswith(("//", "#[", "#!")):
                    cursor += 1
                indent = re.match(r"\s*", lines[cursor]).group(0)
                prefix = "//" if re.match(r"\s*\w+!\s*[(\[{]", lines[cursor]) else "///"
                # После документации теста, прямо над его атрибутами.
                at = next(
                    (index for index in range(head, cursor) if lines[index].strip().startswith("#[")),
                    cursor,
                )
                lines[at:at] = [f"{indent}{prefix} rule: {rule}" for rule in inserts[head]]
        else:
            tree = _parse(source)
            grouped: dict[int, tuple[object, list[str]]] = {}
            for rule, address in wanted:
                _, cls, method = PYTHON_ADDRESS.match(address).groups()
                node = _python_method(tree, cls, method)
                if node is None or not method.startswith("test"):
                    problems.append(f"{rule}: {address}: test is not defined in this file")
                    continue
                before.add((rule, address))
                if rule in PYTHON_MARK.findall(ast.get_docstring(node) or ""):
                    continue
                grouped.setdefault(node.lineno, (node, []))[1].append(rule)
            for _, (node, rules) in sorted(grouped.items(), key=lambda pair: -pair[0]):
                _mark_python(lines, node, rules)
        path.write_bytes("\n".join(lines).replace("\n", newline).encode("utf-8"))
    for rule_path in sorted((root / RULES).rglob("*.md")):
        text = rule_path.read_bytes().decode("utf-8")
        stripped = strip_check(text)
        if stripped != text:
            rule_path.write_bytes(stripped.encode("utf-8"))
    problems.extend(f"address normalized: {line}" for line in renamed)
    return before, marked_pairs(root), problems


# --- Метки отчёта ----------------------------------------------------------


def _rust_module_path(relative: str) -> tuple[str, tuple[str, ...]] | None:
    """Пакет и модульный путь файла по раскладке Cargo; `None` вне `crates/`."""

    parts = relative.split("/")
    if len(parts) < 4 or parts[0] != "crates":
        return None
    package, area, rest = parts[1], parts[2], parts[3:]
    stem = rest[-1][:-3]
    if area == "src":
        modules = [*rest[:-1], stem]
        if stem in ("lib", "main", "mod"):
            modules = modules[:-1]
        return package, tuple(modules)
    if area == "tests":
        # Цель интеграционного теста — файл верхнего уровня `tests/`.
        return f"{package}::{stem}" if len(rest) == 1 else package, ()
    return package, ()


def _declared_targets(crate: Path, package: str) -> list[tuple[Path, str]]:
    """Корни целей `tests/` и `src/bin/`: `[[test]]`/`[[bin]]` и автообнаружение Cargo."""

    manifest = tomllib.loads((crate / "Cargo.toml").read_text(encoding="utf-8"))
    roots: list[tuple[Path, str]] = []
    declared: set[Path] = set()
    for section, directory in (("test", "tests"), ("bin", "src/bin")):
        for target in manifest.get(section, []):
            name = target.get("name")
            path = crate / target.get("path", f"{directory}/{name}.rs")
            if name:
                roots.append((path, f"{package}::{name}"))
                declared.add(path.resolve())
    autotests = manifest.get("package", {}).get("autotests", True)
    for section, directory, automatic in (("test", "tests", autotests), ("bin", "src/bin", True)):
        if not automatic:
            continue
        for path in sorted((crate / directory).glob("*.rs")):
            if path.resolve() not in declared:
                roots.append((path, f"{package}::{path.stem}"))
    return roots


def crate_modules(root: Path) -> dict[str, list[tuple[str, tuple[str, ...]]]]:
    """Модульные пути файлов: обход крейта от корней по `mod`, `#[path]`, `include!`.

    Возвращает для файла список (цель, модули): цель — пакет для `src/` или
    `пакет::файл` для корня интеграционного теста. Файл может входить в
    несколько мест, если его подключают дважды.
    """

    result: dict[str, list[tuple[str, tuple[str, ...]]]] = {}
    for manifest in sorted((root / "crates").glob("*/Cargo.toml")):
        crate = manifest.parent
        package = crate.name
        roots: list[tuple[Path, str]] = []
        for name in ("src/lib.rs", "src/main.rs"):
            if (crate / name).is_file():
                roots.append((crate / name, package))
        roots += _declared_targets(crate, package)
        seen: set[tuple[Path, tuple[str, ...]]] = set()
        queue: list[tuple[Path, str, tuple[str, ...], bool]] = [(path, target, (), True) for path, target in roots]
        while queue:
            path, target, base, owns_dir = queue.pop()
            path = path.resolve()
            if (path, base) in seen or not path.is_file():
                continue
            seen.add((path, base))
            try:
                relative = path.relative_to(root.resolve()).as_posix()
            except ValueError:
                continue
            result.setdefault(relative, []).append((target, base))
            source = path.read_text(encoding="utf-8", errors="replace")
            # Каталог дочерних модулей: у lib.rs, main.rs, mod.rs и корней
            # тестов — свой каталог, у прочих файлов — каталог по имени файла.
            child_dir = path.parent if owns_dir else path.parent / path.stem
            for item in rust_items(source):
                if item.kind == "modfile":
                    modules = base + item.modules + (item.name,)
                    if item.target:
                        # `#[path]` отсчитывается от каталога файла, а внутри
                        # встроенного модуля — от каталога этого модуля.
                        anchor = child_dir.joinpath(*item.modules) if item.modules else path.parent
                        # Файл по `#[path]` держит дочерние модули в своём
                        # каталоге, как `mod.rs`.
                        queue.append((anchor / item.target, target, modules, True))
                    else:
                        directory = child_dir.joinpath(*item.modules)
                        for candidate, owns in ((directory / f"{item.name}.rs", False), (directory / item.name / "mod.rs", True)):
                            if candidate.is_file():
                                queue.append((candidate, target, modules, owns))
                                break
                elif item.kind == "include" and item.target:
                    if item.target.startswith("@crate"):
                        included = crate / item.target[len("@crate"):].lstrip("/")
                    else:
                        included = path.parent / item.target
                    queue.append((included, target, base + item.modules, False))
    return result


class RuleIndex:
    """Правила, названные пометками, по тестам из исходников."""

    def __init__(self, root: Path):
        self.rust: list[tuple[str, tuple[str, ...], tuple[str, ...], str, str | None]] = []
        self.by_name: dict[str, list[tuple[str, tuple[str, ...], tuple[str, ...], str, str | None]]] = {}
        self.python: dict[tuple[str, str, str], set[str]] = {}
        placed = crate_modules(root)
        for rule, address in marked_pairs(root):
            if address.endswith(".py") or ".py::" in address:
                path, test = address.split("::", 1)
                cls, method = test.split(".", 1)
                self.python.setdefault((Path(path).stem, cls, method), set()).add(rule)
                continue
            path, *names = address.split("::")
            if path in placed:
                # Модульный путь известен точно: сопоставление только полное.
                for package, modules in placed[path]:
                    self.rust.append((package, modules, tuple(names), rule, ""))
                continue
            located = _rust_module_path(path)
            if located is None:
                continue
            package, modules = located
            # Файл из `src/`, подключённый через `include!` или `#[path]`,
            # стоит в модуле со своим именем; его имя должно быть в пути.
            stem = Path(path).stem if "/src/" in path and Path(path).stem not in ("lib", "main", "mod") else None
            self.rust.append((package, modules, tuple(names), rule, stem))
        for mark in self.rust:
            self.by_name.setdefault(mark[2][-1], []).append(mark)
        self.unknown = [line for line in stray_marks(root) if "is not defined in arch/rules" in line]

    _cache: dict[Path, "RuleIndex"] = {}

    @classmethod
    def cached(cls, root: Path) -> "RuleIndex":
        if root not in cls._cache:
            cls._cache[root] = cls(root)
        return cls._cache[root]

    def rules_for(self, entry: dict) -> list[str]:
        language = next((item["value"] for item in entry.get("labels", []) if item.get("name") == "language"), "")
        full_name = entry.get("fullName", "")
        if language == "python":
            parts = full_name.split(".")
            return sorted(self.python.get(tuple(parts[-3:]), set())) if len(parts) >= 3 else []
        if language != "rust" or "::" not in full_name:
            return []
        binary = next((item["value"] for item in entry.get("labels", []) if item.get("name") == "suite"), "")
        name = full_name[len(binary) + 2:] if full_name.startswith(binary + "::") else full_name
        segments = tuple(name.split("::"))
        exact, suffix = set(), set()
        for package, modules, names, rule, stem in self.by_name.get(segments[-1], []):
            base = package.split("::")[0]
            if binary != base and not binary.startswith(base + "::"):
                continue
            if binary == package and segments == modules + names:
                exact.add(rule)
            elif stem != "" and segments[-len(names):] == names and (stem is None or stem in segments[: -len(names)]):
                suffix.add(rule)
        return sorted(exact or suffix)


def label_rules(directory: Path, index: RuleIndex) -> int:
    """Поставить метку `rule` записям результатов каталога; вернуть их число."""

    labelled = 0
    for path in sorted(directory.glob("*-result.json")):
        entry = json.loads(path.read_text(encoding="utf-8"))
        rules = index.rules_for(entry)
        labels = [item for item in entry.get("labels", []) if item.get("name") != "rule"]
        labels.extend({"name": "rule", "value": rule} for rule in rules)
        if labels != entry.get("labels", []):
            entry["labels"] = labels
            path.write_text(json.dumps(entry, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        labelled += bool(rules)
    return labelled
