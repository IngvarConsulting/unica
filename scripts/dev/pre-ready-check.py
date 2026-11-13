#!/usr/bin/env python3
"""Проверки ветки перед снятием PR с черновика.

По `git diff` от базы ветки (merge-base с `origin/main`) скрипт находит
затронутые крейты и гоняет:

- `cargo test -p <крейт> --lib` целиком, а у крейта без библиотеки — его
  тестовые цели (`--tests`);
- `cargo clippy -p <крейт> --all-targets --locked -- -D warnings`;
- `cargo fmt --all --check` и `scripts/ci/check-discovered-modules-format.py`;
- полные наборы `tests/ci` и `tests/dev`.

Часть проверок красная и на чистом `main` этого хоста (пути Windows, права
на ссылки, бит исполнения). Поэтому падения сверяются с базой: те же проверки
на дереве базы, в отдельной рабочей копии и своём каталоге сборки. База
считается только для проверок, упавших на ветке, и хранится под
`CARGO_TARGET_DIR` с ключом по sha базы; другая база заменяет кеш. Печатаются
только новые падения; код выхода 0, когда их нет.

Отдельной строкой идут тесты из диффа, которые на этой ОС не исполняются:
исключены `cfg` по ОС, помечены `ignore` по ОС, пропущены по возможности хоста
или `unittest.skip`. Очередь гоняет их на Linux, поэтому их разбирают по коду.

Без `CARGO_TARGET_DIR` скрипт отказывает: сборка в `target/` рабочей копии
не годится для конвейера. Рабочая копия базы и её сборка лежат
в `$CARGO_TARGET_DIR/pre-ready-check/`, поэтому два прогона с одним каталогом
сборки одновременно не запускают.
"""

from __future__ import annotations

import argparse
import ast
import json
import os
import re
import shutil
import subprocess
import sys
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Dict, Iterable, List, Optional, Sequence, Set, Tuple


REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts" / "ci"))
from cargo_target import cargo_target_dir  # noqa: E402

CACHE_DIR_NAME = "pre-ready-check"
CACHE_VERSION = 1
PYTHON_SUITES = ("tests/ci", "tests/dev")
# Крейт вне рабочего пространства: его гоняет `scripts/dev/run-research.py`.
OUTSIDE_WORKSPACE = {"unica-research"}
SKIP_MARKER = "unica-host-capability-skip:"


class Refusal(Exception):
    """Проверку нельзя провести честно: итог был бы ложным."""


# --- Проверки и их итоги ----------------------------------------------------


@dataclass(frozen=True)
class Check:
    """Одна проверка. `key` одинаков на ветке и на базе."""

    key: str
    kind: str  # test | clippy | fmt | modules-fmt | python
    target: str = ""  # пакет, каталог крейта или набор Python
    lib: bool = True

    def command(self, python: str) -> List[str]:
        if self.kind == "test":
            targets = ["--lib"] if self.lib else ["--tests"]
            return ["cargo", "test", "-p", self.target, *targets, "--", "--show-output"]
        if self.kind == "clippy":
            return ["cargo", "clippy", "-p", self.target, "--all-targets", "--locked",
                    "--message-format=short", "--", "-D", "warnings"]
        if self.kind == "fmt":
            return ["cargo", "fmt", "--all", "--check"]
        if self.kind == "modules-fmt":
            return [python, "scripts/ci/check-discovered-modules-format.py"]
        if self.kind == "python":
            return [python, "-m", "unittest", "discover", "-s", self.target, "-v"]
        raise ValueError(self.kind)


@dataclass
class Result:
    failures: List[str]
    # Только для `test` и `python`: что на хосте не исполнилось.
    skipped: List[str] = field(default_factory=list)
    # Падения, причина которых — сбой сборки продукта, а не проверка.
    build: List[str] = field(default_factory=list)


LIBTEST_LINE = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)\b", re.M)
LIBTEST_SUMMARY = re.compile(r"^test result: \w+\. \d+ passed; (\d+) failed", re.M)
SHOW_OUTPUT = re.compile(r"^---- (\S+) std(?:out|err) ----$")
ERROR_LINE = re.compile(r"^error(?:\[E\d+\])?: (.+)$", re.M)
BUILD_SUMMARY = re.compile(r"^(?:could not compile|aborting due to|\d+ previous errors?)")
CLIPPY_LINE = re.compile(r"^(.+?):\d+:\d+: (?:error|warning)(?:\[\w+\])?: (.+)$", re.M)
FMT_LINE = re.compile(r"^Diff in (.+?)(?: at line \d+|:\d+):?\s*$", re.M)
UNITTEST_FAILED = re.compile(r"^(?:FAIL|ERROR): \S+ \(([^)]+)\)", re.M)
# `cargo build` в подготовке набора: `subprocess.CalledProcessError` с этой командой.
CARGO_FAILED = re.compile(
    r"Command '\[['\"]cargo['\"], ['\"]build['\"][^\n]*returned non-zero exit status"
    r"|^error: could not compile", re.M)
UNITTEST_SEPARATOR = re.compile(r"^(?:=|-){70}$", re.M)
UNITTEST_SKIPPED = re.compile(r"\(([\w.]+)\)(?:\n[^\n]*?)? \.\.\. skipped\b", re.M)


def build_errors(output: str, returncode: int) -> List[str]:
    """Ошибки сборки без номеров строк: ключ сравним с базой."""
    errors = sorted({f"сборка: {match.group(1).strip()}" for match in ERROR_LINE.finditer(output)
                     if not BUILD_SUMMARY.match(match.group(1))})
    return errors or [f"сборка или запуск не прошли, код {returncode}"]


def is_build_failure(key: str) -> bool:
    """Ошибка компиляции крейта; упавший при запуске бинарь сюда не входит."""
    return key.startswith("сборка:")


def parse_libtest(output: str, returncode: int) -> Result:
    statuses = {name: status for name, status in LIBTEST_LINE.findall(output)}
    failures = sorted(name for name, status in statuses.items() if status == "FAILED")
    skipped = {name for name, status in statuses.items() if status == "ignored"}
    current = None
    for line in output.splitlines():
        header = SHOW_OUTPUT.match(line)
        if header:
            current = header.group(1)
        elif current and SKIP_MARKER in line:
            skipped.add(current)
    summaries = LIBTEST_SUMMARY.findall(output)
    if returncode != 0:
        if not failures and not summaries:
            failures = build_errors(output, returncode)
        elif not summaries or sum(map(int, summaries)) != len(failures):
            # Бинарь упал посреди прогона или строку итога теста разорвал чужой
            # вывод: часть тестов не исполнена или не учтена.
            failures.append(f"прогон оборвался или итог неполон, код {returncode}")
    return Result(failures, sorted(skipped), [key for key in failures if is_build_failure(key)])


def normalize_path(raw: str, root: Path) -> str:
    text = raw.strip().removeprefix("\\\\?\\")
    path = Path(text)
    if path.is_absolute():
        try:
            path = path.resolve().relative_to(root.resolve())
        except ValueError:
            pass
    return path.as_posix()


def parse_clippy(output: str, returncode: int, root: Path) -> Result:
    # Повторы сохраняются: новый случай той же ошибки в том же файле — новое падение.
    failures = sorted(f"{normalize_path(path, root)}: {message.strip()}"
                      for path, message in CLIPPY_LINE.findall(output))
    if returncode != 0 and not failures:
        failures = build_errors(output, returncode)
    failures = failures if returncode != 0 else []
    return Result(failures, build=[key for key in failures if is_build_failure(key)])


def parse_fmt(output: str, returncode: int, root: Path) -> Result:
    failures = sorted({normalize_path(path, root) for path in FMT_LINE.findall(output)})
    if returncode != 0 and not failures:
        failures = [f"форматирование: отказ без списка файлов, код {returncode}"]
    return Result(failures)


def parse_unittest(output: str, returncode: int) -> Result:
    """Падения по ключу теста; `setUpClass` и `setUpModule` дают ключ класса или модуля."""
    headers = list(UNITTEST_FAILED.finditer(output))
    failures = sorted({header.group(1) for header in headers})
    build = set()
    for index, header in enumerate(headers):
        end = headers[index + 1].start() if index + 1 < len(headers) else len(output)
        # Раздел: строка `---…`, трассировка, следующая строка `===…`.
        dashes = UNITTEST_SEPARATOR.search(output, header.end(), end)
        closing = UNITTEST_SEPARATOR.search(output, dashes.end(), end) if dashes else None
        if CARGO_FAILED.search(output, header.end(), closing.start() if closing else end):
            build.add(header.group(1))
    if returncode != 0 and not failures:
        failures = [f"набор упал без списка тестов, код {returncode}"]
    return Result(failures, sorted(set(UNITTEST_SKIPPED.findall(output))), sorted(build))


def parse(check: Check, output: str, returncode: int, root: Path) -> Result:
    if check.kind == "test":
        return parse_libtest(output, returncode)
    if check.kind == "clippy":
        return parse_clippy(output, returncode, root)
    if check.kind in ("fmt", "modules-fmt"):
        return parse_fmt(output, returncode, root)
    return parse_unittest(output, returncode)


# --- Дифф ---------------------------------------------------------------------


HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@", re.M)


def changed_lines(diff: str) -> Dict[str, Set[int]]:
    """Новые номера строк по файлам из `git diff -U0`; удаление отмечает соседнюю строку."""
    result: Dict[str, Set[int]] = {}
    current: Optional[str] = None
    for line in diff.splitlines():
        if line.startswith("--- "):
            removed = line[4:].strip()
            if removed != "/dev/null":
                result.setdefault(removed.removeprefix("a/"), set())
            continue
        if line.startswith("+++ "):
            name = line[4:].strip()
            current = None if name == "/dev/null" else name.removeprefix("b/")
            if current is not None:
                result.setdefault(current, set())
            continue
        match = HUNK.match(line)
        if match and current is not None:
            start, count = int(match.group(1)), int(match.group(2) or "1")
            result[current].update(range(start, start + count) if count else {max(start, 1)})
    return result


def touched_crates(files: Iterable[str]) -> List[str]:
    """Каталоги крейтов (`crates/<имя>`) затронутых файлов."""
    return sorted({"/".join(name.split("/")[:2]) for name in files
                   if name.startswith("crates/") and name.count("/") >= 2})


# --- cfg и тесты Rust ---------------------------------------------------------


def host_facts(platform: str = sys.platform) -> Dict[str, str]:
    if platform.startswith("win"):
        return {"family": "windows", "os": "windows"}
    if platform == "darwin":
        return {"family": "unix", "os": "macos"}
    return {"family": "unix", "os": "linux"}


CFG_TOKEN = re.compile(r'\s*(?:(\w+)|("(?:\\.|[^"\\])*")|([(),=]))')


def eval_cfg(text: str, host: Dict[str, str]) -> Optional[bool]:
    """Предикат `cfg` на этом хосте: True, False или None, если зависит не от ОС."""
    tokens = []
    position = 0
    while position < len(text):
        match = CFG_TOKEN.match(text, position)
        if not match:
            if text[position:].strip():
                return None
            break
        tokens.append(match.group(1) or match.group(2) or match.group(3))
        position = match.end()
    index = 0

    def expression() -> Optional[bool]:
        nonlocal index
        name = tokens[index]
        index += 1
        if index < len(tokens) and tokens[index] == "(":
            index += 1
            values = []
            while tokens[index] != ")":
                values.append(expression())
                if tokens[index] == ",":
                    index += 1
            index += 1
            if name == "not":
                return None if values[0] is None else not values[0]
            if name == "all":
                return False if False in values else (True if all(v is True for v in values) else None)
            if name == "any":
                return True if True in values else (False if all(v is False for v in values) else None)
            return None
        if index < len(tokens) and tokens[index] == "=":
            value = tokens[index + 1].strip('"')
            index += 2
            if name == "target_os":
                return host["os"] == value
            if name == "target_family":
                return host["family"] == value
            return None
        if name in ("windows", "unix"):
            return host["family"] == name
        if name == "test":
            return True
        return None

    try:
        value = expression()
    except (IndexError, ValueError):
        return None
    return value if index == len(tokens) else None


@dataclass
class RustTest:
    name: str
    start: int
    end: int
    excluded_by: Optional[str] = None  # cfg, который исключает тест здесь
    ignored_by: Optional[str] = None  # cfg_attr(…, ignore), который срабатывает здесь


TEST_ATTRIBUTE = re.compile(r"^(?:test|tokio::test|rstest|test_case|\w+::test)\b")
FN_LINE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(\w+)")
MOD_LINE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{")
CFG_ATTRIBUTE = re.compile(r"^cfg\((.*)\)$", re.S)
CFG_ATTR_IGNORE = re.compile(r"^cfg_attr\((.*),\s*ignore\b.*\)$", re.S)


BRACKETS = str.maketrans("", "", "{}[]")


def strip_code(line: str, state: Dict[str, object]) -> str:
    """Строка без комментариев; в строковых литералах убраны скобки.

    Содержимое литерала остаётся: `target_os = "linux"` нужен разбору `cfg`.
    `state` несёт открытый литерал и блочный комментарий через строки.
    """
    out: List[str] = []
    index = 0
    while index < len(line):
        if state.get("block"):
            end = line.find("*/", index)
            if end < 0:
                break
            state["block"] = False
            index = end + 2
            continue
        closing = state.get("string")
        if closing is not None:
            if closing == '"':
                while index < len(line):
                    char = line[index]
                    if char == "\\":
                        index += 2
                        continue
                    index += 1
                    if char == '"':
                        state["string"] = None
                        out.append('"')
                        break
                    out.append(char.translate(BRACKETS))
            else:
                end = line.find(str(closing), index)
                if end < 0:
                    out.append(line[index:].translate(BRACKETS))
                    break
                out.append(line[index:end].translate(BRACKETS) + '"')
                state["string"] = None
                index = end + len(str(closing))
            continue
        char = line[index]
        if line.startswith("//", index):
            break
        if line.startswith("/*", index):
            state["block"] = True
            index += 2
            continue
        raw = re.match(r'b?r(#*)"', line[index:])
        if raw and (index == 0 or not (line[index - 1].isalnum() or line[index - 1] == "_")):
            state["string"] = '"' + raw.group(1)
            out.append('"')
            index += raw.end()
            continue
        if char == '"':
            state["string"] = '"'
            out.append('"')
            index += 1
            continue
        literal = re.match(r"'(?:\\.[^']*|[^'\\])'", line[index:])
        if char == "'" and literal:
            index += literal.end()
            continue
        out.append(char)
        index += 1
    return "".join(out)


def split_attribute(text: str) -> Optional[Tuple[str, str]]:
    """`#[…]` в начале текста: содержимое и остаток строки; None, если не закрыт."""
    depth = 0
    for index, char in enumerate(text):
        if char == "[":
            depth += 1
        elif char == "]":
            depth -= 1
            if depth == 0:
                return text[2:index].strip(), text[index + 1:].strip()
    return None


def excluding_cfg(attribute: str, host: Dict[str, str]) -> bool:
    match = CFG_ATTRIBUTE.match(attribute)
    return bool(match) and eval_cfg(match.group(1), host) is False


def rust_tests(text: str, host: Dict[str, str]) -> List[RustTest]:
    """Тесты файла с их строками и с тем, что выключает их на этом хосте."""
    file_cfg: Optional[str] = None
    tests: List[RustTest] = []
    state: Dict[str, object] = {"block": False, "string": None}
    depth = 0
    pending: List[Tuple[int, str]] = []
    collecting: Optional[Tuple[int, str]] = None  # строка начала и текст атрибута
    mods: List[Tuple[int, Optional[str]]] = []  # глубина до модуля, исключающий cfg
    open_test: Optional[Tuple[RustTest, int]] = None
    for number, line in enumerate(text.splitlines(), 1):
        code = strip_code(line, state).strip()
        if collecting is not None:
            code = collecting[1] + " " + code
            start = collecting[0]
            collecting = None
        else:
            start = number
        if open_test is None:
            if code.startswith("#![") and depth == 0:
                parts = split_attribute("#" + code[2:])
                if parts and excluding_cfg(parts[0], host):
                    file_cfg = parts[0]
                continue
            while code.startswith("#["):
                parts = split_attribute(code)
                if parts is None:
                    collecting = (start, code)
                    break
                pending.append((start, parts[0]))
                code = parts[1]
            if collecting is not None or not code:
                continue
        before = depth
        depth += code.count("{") - code.count("}")
        if open_test is None:
            fn = FN_LINE.match(code)
            if fn and any(TEST_ATTRIBUTE.match(attribute) for _, attribute in pending):
                excluded = file_cfg or next((cfg for _, cfg in reversed(mods) if cfg), None)
                test = RustTest(fn.group(1), pending[0][0], number, excluded_by=excluded)
                for _, attribute in pending:
                    if test.excluded_by is None and excluding_cfg(attribute, host):
                        test.excluded_by = attribute
                    ignore = CFG_ATTR_IGNORE.match(attribute)
                    if ignore and eval_cfg(ignore.group(1), host) is True:
                        test.ignored_by = attribute
                tests.append(test)
                if depth > before or "{" not in code:
                    open_test = (test, before)
            elif MOD_LINE.match(code):
                cfg = next((attribute for _, attribute in pending if excluding_cfg(attribute, host)), None)
                mods.append((before, cfg))
        elif depth <= open_test[1]:
            open_test[0].end = number
            open_test = None
        pending = []
        while mods and depth <= mods[-1][0]:
            mods.pop()
    return tests


ROOT_FILES = ("lib.rs", "main.rs")


def declared_exclusion(name: str, read: Callable[[str], Optional[str]], host: Dict[str, str]) -> Optional[str]:
    """`cfg` объявления `mod x;` в родительских файлах, исключающий модуль файла здесь."""
    path = Path(name)
    for _ in range(16):
        if path.name in ROOT_FILES or path.parent.name in ("tests", "benches", "examples") and path.parent.parent.name != "src":
            return None
        module, directory = (path.parent.name, path.parent.parent) if path.name == "mod.rs" else (path.stem, path.parent)
        declaration = re.compile(r"((?:#\[[^\n]*\]\s*)+)(?:pub(?:\([^)]*\))?\s+)?mod\s+" + re.escape(module) + r"\s*;")
        candidates = [directory / root for root in ROOT_FILES] if directory.name == "src" else []
        candidates += [directory.with_suffix(".rs"), directory / "mod.rs"]
        parent = next(((candidate, text) for candidate in candidates
                       if (text := read(candidate.as_posix())) is not None), None)
        if parent is None:
            return None
        match = declaration.search(parent[1])
        if match:
            for attribute in re.findall(r"#\[(.*?)\]\s*(?=#\[|$)", match.group(1).strip()):
                if excluding_cfg(attribute.strip(), host):
                    return attribute.strip()
        path = parent[0]
    return None


# --- Тесты Python ---------------------------------------------------------------


def python_tests(text: str) -> List[Tuple[str, int, int]]:
    """`Класс.test_*` с первой и последней строкой, включая декораторы."""
    found = []
    for node in ast.parse(text).body:
        if isinstance(node, ast.ClassDef):
            class_start = min([node.lineno, *(d.lineno for d in node.decorator_list)])
            for item in node.body:
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name.startswith("test"):
                    start = min([item.lineno, *(d.lineno for d in item.decorator_list)])
                    found.append((f"{node.name}.{item.name}", start, item.end_lineno or start))
            # Декоратор класса (`skipUnless`) касается всех его тестов.
            found.append((f"{node.name}.*", class_start, node.lineno))
    return found


# --- Отчёт о тестах, не исполненных на этой ОС ----------------------------------


def unrun_tests(
    lines: Dict[str, Set[int]],
    read: Callable[[str], Optional[str]],
    results: Dict[str, Result],
    crate_packages: Dict[str, str],
    host: Dict[str, str],
) -> List[str]:
    """Тесты из диффа, которые здесь исключены, отключены или пропущены."""
    report: List[str] = []
    skipped_python = {name for key, result in results.items() if key.startswith("python:")
                      for name in result.skipped}
    for name in sorted(lines):
        changed = lines[name]
        if not changed:
            continue
        if name.endswith(".rs") and name.startswith("crates/"):
            text = read(name)
            if text is None:
                continue
            crate = "/".join(name.split("/")[:2])
            package = crate_packages.get(crate, crate.split("/")[1])
            ran = results.get(f"test:{package}")
            declared = declared_exclusion(name, read, host)
            for test in rust_tests(text, host):
                test.excluded_by = test.excluded_by or declared
                if not changed & set(range(test.start, test.end + 1)):
                    continue
                if test.excluded_by:
                    report.append(f"{name}::{test.name} — #[{test.excluded_by}]")
                elif test.ignored_by:
                    report.append(f"{name}::{test.name} — #[{test.ignored_by}]")
                elif ran and any(item == test.name or item.endswith("::" + test.name) for item in ran.skipped):
                    report.append(f"{name}::{test.name} — пропущен на хосте")
        elif name.endswith(".py") and name.startswith(PYTHON_SUITES) and Path(name).name.startswith("test_"):
            text = read(name)
            if text is None:
                continue
            module = Path(name).stem
            try:
                tests = python_tests(text)
            except SyntaxError:
                continue
            touched_classes = {test.split(".")[0] for test, start, end in tests
                               if test.endswith(".*") and changed & set(range(start, end + 1))}
            for test, start, end in tests:
                if test.endswith(".*"):
                    continue
                if not (changed & set(range(start, end + 1)) or test.split(".")[0] in touched_classes):
                    continue
                if f"{module}.{test}" in skipped_python:
                    report.append(f"{name}::{test} — skipped")
    return report


# --- Запуск ---------------------------------------------------------------------


def run_check(check: Check, root: Path, env: Dict[str, str], log_dir: Path) -> Result:
    print(f"  … {' '.join(check.command('python'))}", flush=True)
    completed = subprocess.run(
        check.command(sys.executable), cwd=root, env=env, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace",
    )
    log_dir.mkdir(parents=True, exist_ok=True)
    (log_dir / (re.sub(r"[^\w.-]+", "_", check.key) + ".log")).write_text(completed.stdout, encoding="utf-8")
    return parse(check, completed.stdout, completed.returncode, root)


def git(root: Path, *args: str) -> str:
    completed = subprocess.run(["git", *args], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, encoding="utf-8", errors="replace")
    if completed.returncode != 0:
        raise Refusal(f"git {' '.join(args)}: {completed.stderr.strip()}")
    return completed.stdout


def workspace_packages(root: Path, env: Dict[str, str]) -> Dict[str, Tuple[str, bool]]:
    """Каталог крейта → (пакет, есть ли библиотека)."""
    completed = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=root, env=env,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8")
    if completed.returncode != 0:
        raise Refusal(f"cargo metadata: {completed.stderr.strip()}")
    packages = {}
    for package in json.loads(completed.stdout)["packages"]:
        directory = Path(package["manifest_path"]).resolve().parent
        try:
            relative = directory.relative_to(root.resolve()).as_posix()
        except ValueError:
            continue
        lib = any(kind in ("lib", "rlib", "proc-macro") for target in package["targets"] for kind in target["kind"])
        packages[relative] = (package["name"], lib)
    return packages


def plan(crates: Sequence[str], packages: Dict[str, Tuple[str, bool]]) -> Tuple[List[Check], List[str]]:
    checks: List[Check] = []
    notes: List[str] = []
    for crate in crates:
        if crate not in packages:
            if crate.split("/")[1] in OUTSIDE_WORKSPACE:
                notes.append(f"{crate}: вне рабочего пространства, его гоняет scripts/dev/run-research.py")
            continue
        name, lib = packages[crate]
        checks.append(Check(f"test:{name}", "test", name, lib))
        checks.append(Check(f"clippy:{name}", "clippy", name))
    checks.append(Check("fmt", "fmt"))
    checks.append(Check("modules-fmt", "modules-fmt"))
    checks.extend(Check(f"python:{suite}", "python", suite) for suite in PYTHON_SUITES)
    return checks, notes


def load_cache(path: Path, base: str) -> Dict[str, List[str]]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    if data.get("version") != CACHE_VERSION or data.get("base") != base or data.get("platform") != sys.platform:
        return {}
    return {key: list(value) for key, value in data.get("checks", {}).items()}


def save_cache(path: Path, base: str, checks: Dict[str, List[str]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"version": CACHE_VERSION, "base": base, "platform": sys.platform,
                                "checks": checks}, ensure_ascii=False, indent=1), encoding="utf-8")


def baseline(checks: Sequence[Check], base: str, cache_dir: Path, env: Dict[str, str]) -> Dict[str, List[str]]:
    """Падения тех же проверок на дереве базы; недостающие досчитываются и кешируются."""
    cache_path = cache_dir / "baseline.json"
    cached = load_cache(cache_path, base)
    missing = [check for check in checks if check.key not in cached]
    if not missing:
        return cached
    tree = cache_dir / "base"
    print(f"База {base[:12]}: считаю {len(missing)} проверок в {tree}", flush=True)
    if tree.exists():
        subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=REPO_ROOT)
        shutil.rmtree(tree, ignore_errors=True)
        git(REPO_ROOT, "worktree", "prune")
    git(REPO_ROOT, "worktree", "add", "--detach", str(tree), base)
    base_env = {**env, "CARGO_TARGET_DIR": str(cache_dir / "base-build")}
    try:
        for check in missing:
            if check.kind in ("test", "clippy") and not crate_exists_on_base(tree, check.target):
                cached[check.key] = []
                continue
            cached[check.key] = run_check(check, tree, base_env, cache_dir / "logs-base").failures
            save_cache(cache_path, base, cached)
    finally:
        subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=REPO_ROOT)
    save_cache(cache_path, base, cached)
    return cached


def crate_exists_on_base(tree: Path, package: str) -> bool:
    return any(
        re.search(rf'^name\s*=\s*"{re.escape(package)}"', manifest.read_text(encoding="utf-8"), re.M)
        for manifest in (tree / "crates").glob("*/Cargo.toml")
    )


def related(first: str, second: str) -> bool:
    """Один тест Python или тест и его класс (модуль): `setUpClass` падает ключом класса."""
    return first == second or first.startswith(second + ".") or second.startswith(first + ".")


def covered(name: str, known: Iterable[str]) -> bool:
    """Падение Python известно базе: тот же тест или его класс (модуль) упал там целиком.

    Обратное не действует: `setUpClass` на ветке не исполнил ни одного метода,
    и падение одного из них на базе не говорит об остальных. Такой класс
    разбирает повтор: он исполняет методы, и их падения сверяются по своим ключам.
    """
    return any(name == other or name.startswith(other + ".") for other in known)


def new_failures(current: Dict[str, Result], base: Dict[str, List[str]]) -> Dict[str, List[str]]:
    result: Dict[str, List[str]] = {}
    for key, outcome in current.items():
        known = base.get(key, [])
        fresh = sorted((Counter(outcome.failures) - Counter(known)).elements())
        if key.startswith("python:"):
            fresh = [name for name in fresh if not covered(name, known)]
        if fresh:
            result[key] = fresh
    return result


SYNTHETIC = ("сборка", "прогон оборвался", "набор упал", "форматирование:")


def recheck(check: Check, fresh: List[str], root: Path, env: Dict[str, str],
            log_dir: Path) -> Tuple[Result, List[str]]:
    """Новые падения тестов повторяются один раз: (падения повтора, нестабильные).

    На Windows часть тестов изредка падает на занятом файле. Тест, ни одно
    падение которого не повторилось, печатается отдельно и итог не краснит.
    Повтор класса исполняет все его методы: их падения возвращаются своими
    ключами, и вызывающий заново сверяет их с базой.
    """
    names = sorted({name for name in fresh if not name.startswith(SYNTHETIC)})
    if check.kind not in ("test", "python") or not names:
        return Result(fresh), []
    if check.kind == "python":
        command = [sys.executable, "-m", "unittest", "-v", *names]
        env = {**env, "PYTHONPATH": os.pathsep.join(filter(None, [str(root / check.target),
                                                                   env.get("PYTHONPATH")]))}
    else:
        command = check.command(sys.executable)[:-1] + ["--show-output", "--exact", *names]
    print(f"  … повтор {len(names)} новых падений {check.key}", flush=True)
    completed = subprocess.run(command, cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, encoding="utf-8", errors="replace")
    log_dir.mkdir(parents=True, exist_ok=True)
    (log_dir / (re.sub(r"[^\w.-]+", "_", check.key) + ".rerun.log")).write_text(completed.stdout, encoding="utf-8")
    repeated = parse(check, completed.stdout, completed.returncode, root)
    if any(name.startswith(SYNTHETIC) for name in repeated.failures):
        return Result(fresh), []  # повтор сам не прошёл: доверять нечему
    unstable = [name for name in names if not any(related(failure, name) for failure in repeated.failures)]
    still: List[str] = [name for name in fresh if name.startswith(SYNTHETIC)]
    build: Set[str] = set()
    for failure in repeated.failures:
        if any(failure == name or failure.startswith(name + ".") for name in names):
            still.append(failure)  # сам повторённый тест или метод повторённого класса
            if failure in repeated.build:
                build.add(failure)
            continue
        # Класс или модуль упал целиком: повторённые под ним тесты не исполнились,
        # и их падения остаются своими ключами, а не ключом класса.
        for name in names:
            if name.startswith(failure + "."):
                still.append(name)
                if failure in repeated.build:
                    build.add(name)
    return Result(sorted(set(still)), build=sorted(build)), unstable


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", default="origin/main", help="ветка базы, по умолчанию origin/main")
    args = parser.parse_args(argv)

    target = os.environ.get("CARGO_TARGET_DIR")
    if not target:
        print("pre-ready-check: задайте CARGO_TARGET_DIR — свой каталог сборки исполнителя", file=sys.stderr)
        return 2
    cache_dir = cargo_target_dir(REPO_ROOT).resolve() / CACHE_DIR_NAME
    env = {**os.environ, "PYTHONUTF8": "1"}
    try:
        base = git(REPO_ROOT, "merge-base", args.base, "HEAD").strip()
        lines = changed_lines(git(REPO_ROOT, "-c", "core.quotePath=false", "diff", "-U0", "--no-color", "--no-renames", base))
        dirty = git(REPO_ROOT, "status", "--porcelain", "--untracked-files=no").strip()
        packages = workspace_packages(REPO_ROOT, env)
    except Refusal as error:
        print(f"pre-ready-check: {error}", file=sys.stderr)
        return 2

    checks, notes = plan(touched_crates(lines), packages)
    crate_names = sorted(key.split(":", 1)[1] for key in (c.key for c in checks) if key.startswith("test:"))
    print(f"База {base[:12]} ({args.base}); крейты: {', '.join(crate_names) or 'нет'}")
    if dirty:
        print("Рабочее дерево с незакоммиченными правками: итог относится к дереву, а не к HEAD.")
    for note in notes:
        print(f"  {note}")

    current: Dict[str, Result] = {}
    for check in checks:
        current[check.key] = run_check(check, REPO_ROOT, env, cache_dir / "logs")
    failed = [check for check in checks if current[check.key].failures]
    try:
        base_failures = baseline(failed, base, cache_dir, env) if failed else {}
    except Refusal as error:
        print(f"pre-ready-check: база не посчитана: {error}", file=sys.stderr)
        return 2
    fresh = new_failures(current, base_failures)
    unstable: Dict[str, List[str]] = {}
    build: Dict[str, Set[str]] = {key: set(result.build) for key, result in current.items()}
    for check in checks:
        if check.key in fresh:
            repeated, flaky = recheck(check, fresh[check.key], REPO_ROOT, env, cache_dir / "logs")
            build[check.key] = set(repeated.build)
            stable = new_failures({check.key: repeated}, base_failures).get(check.key, [])
            if flaky:
                unstable[check.key] = flaky
            if stable:
                fresh[check.key] = stable
            else:
                del fresh[check.key]

    print()
    for check in checks:
        known_on_base = base_failures.get(check.key, [])
        known = sum(1 for failure in set(current[check.key].failures)
                    if failure in known_on_base
                    or (check.key.startswith("python:") and covered(failure, known_on_base)))
        status = f"новых падений: {len(fresh[check.key])}" if check.key in fresh else "ok"
        if known:
            status += f", падают и на базе: {known}"
        print(f"[{status}] {check.key}")
        for failure in fresh.get(check.key, []):
            print(f"    {'сбой сборки: ' if failure in build[check.key] else ''}{failure}")
        for failure in unstable.get(check.key, []):
            print(f"    нестабилен, прошёл при повторе: {failure}")
    crate_packages = {crate: name for crate, (name, _) in packages.items()}

    def read(name: str) -> Optional[str]:
        try:
            return (REPO_ROOT / name).read_text(encoding="utf-8")
        except OSError:
            return None

    unrun = unrun_tests(lines, read, current, crate_packages, host_facts())
    print()
    builds = sum(failure in build[key] for key, failures in fresh.items() for failure in failures)
    count = sum(map(len, fresh.values()))
    print(f"Новые падения: {count if fresh else 'нет'}"
          + (f" (из них сбой сборки: {builds} — сборка продукта, а не проверка)" if builds else ""))
    print(f"Не исполнены на этой ОС, разобрать по коду: {'; '.join(unrun) if unrun else 'нет'}")
    print(f"Журналы: {cache_dir / 'logs'}")
    return 1 if fresh else 0


if __name__ == "__main__":
    raise SystemExit(main())
