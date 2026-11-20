#!/usr/bin/env python3
"""Проверки ветки перед снятием PR с черновика.

По `git diff` от базы ветки (merge-base с `origin/main`) скрипт находит
затронутые крейты и крейты рабочего пространства, которые от них зависят
(обратные зависимости по `cargo metadata`, включая dev-зависимости), и гоняет:

- `cargo test -p <крейт> --tests --no-fail-fast`: библиотеку, интеграционные
  цели и бинари, сначала у затронутых крейтов, затем у зависимых;
- Clippy всего workspace, как ворота PR, когда затронут хоть один крейт;
- `scripts/ci/check-discovered-modules-format.py` для всех Rust-файлов крейтов;
- `scripts/ci/check-version-contract.py`;
- `scripts/ci/open_marks.py pr`: пометки незакрытого называют открытые задачи
  (нужен `gh`); с `--description-file` — и не те, что закрывает этот PR;
- полные наборы `tests/ci` и `tests/dev`;
- с `--description-file` — `scripts/ci/removed_tests.py` по описанию PR.
  Без описания исчезнувшие тесты печатаются напоминанием: эти строки
  `Removes-test:` нужны в описании PR.

Состав сверяется с воротами PR в `.github/workflows/ci.yml`: у каждого их шага
в `PR_GATES` записано, какая проверка гейта его покрывает. Шаг, которого там
нет, печатается как неизвестный гейту; то, чего гейт не исполняет на хосте,
и шаги только очереди названы отдельно от выполненного.

Часть проверок красная и на чистом `main` этого хоста (пути Windows, права
на ссылки, бит исполнения). Поэтому падения сверяются с базой: те же проверки
на дереве базы, в отдельной рабочей копии и своём каталоге сборки. База
считается только для проверок, упавших на ветке, и хранится под
`CARGO_TARGET_DIR` с ключом по sha базы; другая база заменяет кеш. Печатаются
только новые падения; код выхода 0, когда их нет.

Итог печатается только по досчитанным проверкам. Проверка досчитана, когда
её прогон закончился сам: у `unittest` есть строка `Ran N tests`, у `cargo`
код 0 или его собственная строка `error:`. Недосчитанная проверка ветки
снимает итог: вместо «Новые падения» печатается, чего не хватает, код выхода 3.
Недосчитанная база в кеш не попадает и считается заново.

Отдельной строкой идут тесты из диффа, которые на этой ОС не исполняются:
исключены `cfg` по ОС, помечены `ignore` по ОС, пропущены по возможности хоста
или `unittest.skip`. Очередь гоняет их на Linux, поэтому их разбирают по коду.

Без `CARGO_TARGET_DIR` скрипт отказывает: сборка в `target/` рабочей копии
не годится для конвейера. Отказывает он и с каталогом сборки, который собирал
другую рабочую копию: Cargo решает, что пересобрать, по времени файлов, и
может проверить код той копии вместо этой. Копию-хозяина каталог помнит
в файле `unica-worktree`; сборки обычным `cargo` из другой копии видны
по `CARGO_MANIFEST_DIR` в файлах `.d` крейтов рабочего пространства. Рабочая копия базы и её сборка лежат
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
import time
from collections import Counter
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Dict, Iterable, Iterator, List, Mapping, Optional, Sequence, Set, Tuple


REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts" / "ci"))
from cargo_target import cargo_target_dir  # noqa: E402

CACHE_DIR_NAME = "pre-ready-check"
# 2: в кеше только досчитанные проверки базы.
# 3: авария сборки на базе больше не кешируется как падения; кешам версии 2 не верим.
CACHE_VERSION = 3
INCOMPLETE_EXIT = 3
PYTHON_SUITES = ("tests/ci", "tests/dev")
# Крейт вне рабочего пространства: его гоняет `scripts/dev/run-research.py`.
OUTSIDE_WORKSPACE = {"unica-research"}
SKIP_MARKER = "unica-host-capability-skip:"
# Замок рабочих копий в общем `.git`: создание и удаление копии базы
# правят `.git/worktrees`, и параллельные прогоны из разных копий
# одного репозитория мешают друг другу.
WORKTREE_LOCK = "pre-ready-check-worktrees.lock"
LOCK_WAIT_SECONDS = 300.0
# Файл в каталоге сборки с корнем рабочей копии, которая им пользуется.
TARGET_OWNER = "unica-worktree"
MANIFEST_DIR_DEP = re.compile(r"^# env-dep:CARGO_MANIFEST_DIR=(.+)$", re.MULTILINE)


class Refusal(Exception):
    """Проверку нельзя провести честно: итог был бы ложным."""


# --- Проверки и их итоги ----------------------------------------------------


@dataclass(frozen=True)
class Check:
    """Одна проверка. `key` одинаков на ветке и на базе."""

    key: str
    kind: str  # test | clippy | fmt | version-contract | open-marks | removed-tests | python
    target: str = ""  # пакет, каталог крейта, набор Python или файл описания PR
    base: str = ""  # для `removed-tests`: от какой ревизии считать исчезнувшее

    def command(self, python: str) -> List[str]:
        if self.kind == "test":
            # `--no-fail-fast`: упавшая цель не отменяет прогон остальных.
            return ["cargo", "test", "-p", self.target, "--tests", "--no-fail-fast", "--", "--show-output"]
        if self.kind == "clippy":
            # Как ворота PR: весь workspace, все цели и возможности.
            return ["cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--locked",
                    "--message-format=short", "--", "-D", "warnings"]
        if self.kind == "version-contract":
            return [python, "scripts/ci/check-version-contract.py"]
        if self.kind == "open-marks":
            description = Path(self.target).read_text(encoding="utf-8") if self.target else ""
            return [python, "scripts/ci/open_marks.py", "pr", "--description", description]
        if self.kind == "removed-tests":
            return [python, "scripts/ci/removed_tests.py", "--base", self.base, "--description-file", self.target]
        if self.kind == "fmt":
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
    # Прогон закончился сам, а не оборван: по неполному итогу не судят.
    complete: bool = True
    # Чего проверка не проверила, потому что сборка не состоялась: ни один
    # тестовый бинарь не запустился. Это не падение и не успех, а «не проверено».
    unverified: List[str] = field(default_factory=list)


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
UNITTEST_RAN = re.compile(r"^Ran \d+ tests? in ", re.M)
CARGO_RUNNING = re.compile(r"^[ \t]+Running ", re.M)
CARGO_RERUN = re.compile(r"^error: test failed, to rerun pass `([^`]+)`", re.M)
CARGO_TARGETS_FAILED = re.compile(r"^error: \d+ targets? failed:$", re.M)
REMOVED_NONE = "no undeclared removed tests"
# Проверки без сверки с базой: каждый их отказ — отказ ворот PR.
BASELESS = ("removed-tests", "open-marks")
REMOVED_DECLARE = re.compile(r"declare '([^']+)'")


def cargo_finished(output: str, returncode: int) -> bool:
    """`cargo` закончил сам: успех или его собственная строка `error:`.

    Убитый процесс не успевает её напечатать, и его код ничего не говорит:
    на Windows снятая задача выходит с кодом 1, как обычный отказ.
    """
    return returncode == 0 or ERROR_LINE.search(output) is not None


def build_errors(output: str, returncode: int) -> List[str]:
    """Ошибки сборки без номеров строк: ключ сравним с базой."""
    errors = sorted({f"сборка: {match.group(1).strip()}" for match in ERROR_LINE.finditer(output)
                     if not BUILD_SUMMARY.match(match.group(1))})
    return errors or [f"сборка или запуск не прошли, код {returncode}"]


def is_build_failure(key: str) -> bool:
    """Ошибка компиляции крейта; упавший при запуске бинарь сюда не входит."""
    return key.startswith("сборка:")


def parse_libtest(output: str, returncode: int) -> Result:
    """Падения `cargo test` по целям: у каждого бинаря свой раздел от строки `Running`.

    Цель без строки итога упала сама (аварийный выход, переполнение стека):
    её падение — ключ `цель <...> упала без итога`, даже если именованных
    падений нет. Прогон досчитан по коду 0 или по заключительной строке
    `error: N targets failed:`, которую `--no-fail-fast` печатает в конце;
    строка `error: test failed` после одной цели об этом не говорит. Сбой
    сборки до первой цели досчитан по строке `error:` самого `cargo`.
    """
    starts = [match.start() for match in CARGO_RUNNING.finditer(output)]
    sections = [output[start:end] for start, end in zip(starts, starts[1:] + [len(output)])] or [output]
    failures: List[str] = []
    skipped: Set[str] = set()
    for index, section in enumerate(sections):
        statuses = LIBTEST_LINE.findall(section)
        named = sorted(name for name, status in statuses if status == "FAILED")
        failures.extend(named)
        skipped.update(name for name, status in statuses if status == "ignored")
        current = None
        for line in section.splitlines():
            header = SHOW_OUTPUT.match(line)
            if header:
                current = header.group(1)
            elif current and SKIP_MARKER in line:
                skipped.add(current)
        summaries = LIBTEST_SUMMARY.findall(section)
        if returncode == 0 or (not starts and not named and not summaries):
            continue
        rerun = CARGO_RERUN.search(section)
        if not summaries and (rerun or index < len(sections) - 1):
            target = rerun.group(1) if rerun else section.splitlines()[0].strip()
            failures.append(f"цель {target} упала без итога")
        elif summaries and sum(map(int, summaries)) != len(named):
            # Строку итога теста разорвал чужой вывод: часть падений не учтена.
            failures.append(f"прогон оборвался или итог неполон, код {returncode}")
        elif not summaries and not starts:
            failures.append(f"прогон оборвался или итог неполон, код {returncode}")
    if returncode == 0:
        finished = True
    elif starts:
        finished = CARGO_TARGETS_FAILED.search(output) is not None
    else:
        finished = ERROR_LINE.search(output) is not None
    # Ни одной строки `Running` при ненулевом коде: `cargo` собирает все цели и
    # запускает их только после удачной сборки, поэтому ни один тест не исполнился.
    # Это «не проверено», а не падение теста: судим по улике, а не по тексту ошибки
    # компилятора. Ошибку в коде правки покажет Clippy, он компилирует то же.
    unverified = build_errors(output, returncode) if returncode != 0 and not starts and finished else []
    return Result(sorted(failures), sorted(skipped), [], finished, unverified)


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
    return Result(failures, build=[key for key in failures if is_build_failure(key)],
                  complete=cargo_finished(output, returncode))


def parse_fmt(output: str, returncode: int, root: Path) -> Result:
    failures = sorted({normalize_path(path, root) for path in FMT_LINE.findall(output)})
    finished = returncode == 0 or bool(failures) or ERROR_LINE.search(output) is not None
    if returncode != 0 and not failures:
        failures = [f"форматирование: отказ без списка файлов, код {returncode}"]
    return Result(failures, complete=finished)


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
    # Тест, который упал на сборке продукта (`cargo build` в подготовке), ничего
    # не проверил: он «не проверен», а не упал.
    failures = [failure for failure in failures if failure not in build]
    if returncode != 0 and not failures and not build:
        failures = [f"набор упал без списка тестов, код {returncode}"]
    return Result(failures, sorted(set(UNITTEST_SKIPPED.findall(output))), sorted(build),
                  UNITTEST_RAN.search(output) is not None, sorted(build))


def parse_script(output: str, returncode: int) -> Result:
    """Скрипт-страж: отказ — его строки `::error::` или все непустые, без них — код выхода.

    Трассировка Python — сбой самого стража (нет `gh`, сети), а не его отказ.
    """
    if returncode == 0:
        return Result([])
    if "Traceback (most recent call last)" in output:
        return Result([f"страж не досчитал: {output.strip().splitlines()[-1]}"], complete=False)
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    errors = [line.removeprefix("::error::") for line in lines if line.startswith("::error::")]
    return Result(sorted(set(errors or lines)) or [f"отказ без пояснения, код {returncode}"])


def parse_removed(output: str, returncode: int) -> Result:
    """`removed_tests.py`: каждая строка — исчезнувший необъявленный тест; код 2 — git не прочитал ревизии."""
    if returncode == 2:
        return Result([f"сверка не проведена: {output.strip()}"], complete=False)
    return Result(sorted({line.strip().removeprefix("::error::") for line in output.splitlines()
                          if line.strip() and line.strip() != REMOVED_NONE}) if returncode else [])


def parse(check: Check, output: str, returncode: int, root: Path) -> Result:
    if check.kind == "test":
        return parse_libtest(output, returncode)
    if check.kind == "clippy":
        return parse_clippy(output, returncode, root)
    if check.kind == "fmt":
        return parse_fmt(output, returncode, root)
    if check.kind in ("version-contract", "open-marks"):
        return parse_script(output, returncode)
    if check.kind == "removed-tests":
        return parse_removed(output, returncode)
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


def load_queue_tests():
    """`scripts/ci/queue-tests.py`: признак «правка задевает всю сборку» живёт там."""
    import importlib.util  # noqa: PLC0415
    spec = importlib.util.spec_from_file_location("queue_tests", REPO_ROOT / "scripts" / "ci" / "queue-tests.py")
    if "queue_tests" in sys.modules:
        return sys.modules["queue_tests"]
    module = importlib.util.module_from_spec(spec)
    # `dataclass` ищет модуль класса в `sys.modules`.
    sys.modules["queue_tests"] = module
    spec.loader.exec_module(module)
    return module


def whole_build_reason(files: Sequence[str], base: str, members: Set[str], root: Path = REPO_ROOT) -> Optional[str]:
    """Почему правка задевает всю сборку, как решает очередь, или `None`.

    Признак спрашивается у очереди, а не описывается здесь второй раз: правка
    `Cargo.lock` только в записях крейтов рабочей области всей сборки не задевает.
    Не прочитав `Cargo.lock` ревизий, считаем, что задевает: как очередь при сбое.
    """
    queue = load_queue_tests()
    internal = False
    if "Cargo.lock" in files:
        try:
            internal = queue.lock_keeps_external(base, "HEAD", members, root)
        except (OSError, subprocess.CalledProcessError, ValueError):
            internal = False
    return queue.whole_build(list(files), lock_is_internal=internal)


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
    # Скрипт, который исполняет отключённый тест: его называет причина `ignore`.
    runner: Optional[str] = None


TEST_ATTRIBUTE = re.compile(r"^(?:test|tokio::test|rstest|test_case|\w+::test)\b")
FN_LINE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(\w+)")
MOD_LINE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{")
CFG_ATTRIBUTE = re.compile(r"^cfg\((.*)\)$", re.S)
CFG_ATTR_IGNORE = re.compile(r"^cfg_attr\((.*),\s*ignore\b.*\)$", re.S)
# Причина `#[ignore = "<вид>: …; run <скрипт>"]` называет исполнителя теста.
IGNORE_RUNNER = re.compile(r'\bignore\s*=\s*"[^"]*?\brun ((?:scripts|tests)/[\w./-]+\.(?:py|sh|ps1))')


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
                    runner = IGNORE_RUNNER.search(attribute)
                    if runner and (attribute.startswith("ignore") or test.ignored_by == attribute):
                        test.runner = runner.group(1)
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
) -> Tuple[List[str], List[str]]:
    """Тесты из диффа, которые здесь исключены, отключены или пропущены.

    Второй список — отключённые тесты, которые исполняет другой инструмент:
    его путь называет причина `#[ignore]`, и файл по этому пути есть.
    Устаревшая ссылка тест не прячет: он остаётся в первом списке.
    """
    report: List[str] = []
    elsewhere: List[str] = []
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
                if test.runner and not test.excluded_by and read(test.runner) is not None:
                    elsewhere.append(f"{name}::{test.name} — {test.runner}")
                elif test.excluded_by:
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
    return report, elsewhere


# --- Каталог сборки одной рабочей копии ----------------------------------------


def same_path(first: Path, second: Path) -> bool:
    return os.path.normcase(os.path.abspath(first)) == os.path.normcase(os.path.abspath(second))


def built_copies(target: Path, root: Path) -> List[Path]:
    """Корни других копий этого репозитория, чьи крейты собраны в `target`.

    Видны крейты, которые читают `CARGO_MANIFEST_DIR`: rustc пишет его в `.d`.
    Чужой корень — путь вида `<корень>/crates/<крейт>`, где `<крейт>` есть
    и в этой копии; каталоги сторонних зависимостей так не выглядят.
    Просматриваются `<профиль>/deps` и `<триплет>/<профиль>/deps`: с
    `CARGO_BUILD_TARGET` Cargo кладёт сборку под каталог триплета.
    """
    crates = {path.parent.name for path in (root / "crates").glob("*/Cargo.toml")}
    found: List[Path] = []
    for pattern in ("*/deps/*.d", "*/*/deps/*.d"):
        for dep_info in sorted(target.glob(pattern)):
            try:
                text = dep_info.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            for match in MANIFEST_DIR_DEP.finditer(text):
                manifest = Path(match.group(1).strip().replace("\\\\", "\\"))
                if manifest.parent.name != "crates" or manifest.name not in crates:
                    continue
                copy = manifest.parent.parent
                if not same_path(copy, root) and not any(same_path(copy, known) for known in found):
                    found.append(copy)
    return found


def claim_target(target: Path, root: Path) -> None:
    """Закрепляет каталог сборки за копией `root` или отказывает, если он собирал другую."""
    owner_file = target / TARGET_OWNER
    try:
        owner = Path(owner_file.read_text(encoding="utf-8").strip())
    except OSError:
        owner = None
    others = [owner] if owner is not None and not same_path(owner, root) else []
    others += [copy for copy in built_copies(target, root) if not any(same_path(copy, o) for o in others)]
    if others:
        raise Refusal(
            f"в каталоге сборки {target} собирали другую рабочую копию: {', '.join(map(str, others))}. "
            "Cargo может счесть её артефакты свежими, и проверка пройдёт по её коду, а не по вашему. "
            "Задайте этой копии свой CARGO_TARGET_DIR; если та копия закончена, удалите каталог сборки целиком."
        )
    target.mkdir(parents=True, exist_ok=True)
    owner_file.write_text(f"{root}\n", encoding="utf-8")


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
        said = completed.stderr.strip() or completed.stdout.strip() or "git ничего не вывел"
        raise Refusal(f"git {' '.join(args)}: код {completed.returncode}: {said}")
    return completed.stdout


def try_lock(descriptor: int) -> bool:
    """Блокировка ОС на первый байт файла; её снимает и смерть процесса."""
    try:
        if os.name == "nt":
            import msvcrt

            os.lseek(descriptor, 0, os.SEEK_SET)
            msvcrt.locking(descriptor, msvcrt.LK_NBLCK, 1)
        else:
            import fcntl

            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        return False
    return True


def unlock(descriptor: int) -> None:
    if os.name == "nt":
        import msvcrt

        os.lseek(descriptor, 0, os.SEEK_SET)
        msvcrt.locking(descriptor, msvcrt.LK_UNLCK, 1)
    else:
        import fcntl

        fcntl.flock(descriptor, fcntl.LOCK_UN)


def lock_holder(path: Path) -> str:
    """Кто держит замок: держатель пишет себя после первого, заблокированного байта."""
    try:
        with open(path, "rb") as handle:
            handle.seek(1)
            text = handle.read(512).decode("utf-8", errors="replace").strip("\0 \n")
    except OSError:
        text = ""
    return text or "неизвестный прогон"


@contextmanager
def worktree_lock(root: Path, wait: float = LOCK_WAIT_SECONDS, poll: float = 0.5) -> Iterator[None]:
    """Замок на создание и удаление рабочей копии базы в общем `.git` репозитория.

    Файл замка не удаляется: держит блокировка ОС, и прерванный прогон
    не оставляет её за собой.
    """
    common = Path(git(root, "rev-parse", "--git-common-dir").strip())
    path = (common if common.is_absolute() else root / common) / WORKTREE_LOCK
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT, 0o666)
    try:
        deadline = time.monotonic() + wait
        announced = False
        while not try_lock(descriptor):
            holder = lock_holder(path)
            if time.monotonic() >= deadline:
                raise Refusal(
                    f"рабочие копии репозитория правит другой прогон pre-ready-check ({holder}); "
                    f"ждал {wait:.0f} с. Дождитесь его конца и повторите; если он завис, остановите его")
            if not announced:
                print(f"  … жду замок рабочих копий: его держит {holder}", flush=True)
                announced = True
            time.sleep(poll)
        try:
            os.lseek(descriptor, 1, os.SEEK_SET)
            os.write(descriptor, f"pid {os.getpid()}, копия {root}".encode("utf-8").ljust(512, b" "))
            yield
        finally:
            unlock(descriptor)
    finally:
        os.close(descriptor)


def add_base_tree(root: Path, tree: Path, base: str) -> None:
    """Свежая рабочая копия базы на месте прежней."""
    with worktree_lock(root):
        if tree.exists() or tree.is_symlink():
            subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=root,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            shutil.rmtree(tree, ignore_errors=True)
        git(root, "worktree", "prune")
        git(root, "worktree", "add", "--detach", str(tree), base)


def remove_base_tree(root: Path, tree: Path) -> None:
    with worktree_lock(root):
        subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=root,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


@dataclass(frozen=True)
class Package:
    name: str
    # Пакеты рабочего пространства, от которых он зависит, с dev и build.
    depends: Tuple[str, ...] = ()


def workspace_packages(root: Path, env: Dict[str, str]) -> Dict[str, Package]:
    """Каталог крейта → пакет и его зависимости внутри рабочего пространства."""
    completed = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=root, env=env,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8")
    if completed.returncode != 0:
        raise Refusal(f"cargo metadata: {completed.stderr.strip()}")
    metadata = json.loads(completed.stdout)
    members = {package["name"] for package in metadata["packages"]}
    packages = {}
    for package in metadata["packages"]:
        directory = Path(package["manifest_path"]).resolve().parent
        try:
            relative = directory.relative_to(root.resolve()).as_posix()
        except ValueError:
            continue
        depends = sorted({dependency["name"] for dependency in package["dependencies"]
                          if dependency.get("path") and dependency["name"] in members})
        packages[relative] = Package(package["name"], tuple(depends))
    return packages


def dependents(names: Iterable[str], packages: Dict[str, Package]) -> List[str]:
    """Пакеты, которые зависят от `names` прямо или через другие, без самих `names`."""
    reverse: Dict[str, Set[str]] = {}
    for package in packages.values():
        for dependency in package.depends:
            reverse.setdefault(dependency, set()).add(package.name)
    seen = set(names)
    queue = list(seen)
    while queue:
        for user in sorted(reverse.get(queue.pop(), ())):
            if user not in seen:
                seen.add(user)
                queue.append(user)
    return sorted(seen - set(names))


@dataclass
class Plan:
    checks: List[Check]
    notes: List[str]
    touched: List[str]
    users: List[str]


def plan(crates: Sequence[str], packages: Dict[str, Package], base: str = "",
         description: Optional[str] = None) -> Plan:
    """Сначала тесты затронутых крейтов, затем тесты зависимых, Clippy workspace, затем общее."""
    checks: List[Check] = []
    notes: List[str] = []
    touched: List[str] = []
    for crate in crates:
        if crate not in packages:
            if crate.split("/")[1] in OUTSIDE_WORKSPACE:
                notes.append(f"{crate}: вне рабочего пространства, его гоняет scripts/dev/run-research.py")
            continue
        name = packages[crate].name
        touched.append(name)
        checks.append(Check(f"test:{name}", "test", name))
    users = dependents(touched, packages)
    checks.extend(Check(f"test:{name}", "test", name) for name in users)
    if touched:
        checks.append(Check("clippy", "clippy"))
    checks.append(Check("fmt", "fmt"))
    checks.append(Check("version-contract", "version-contract"))
    checks.append(Check("open-marks", "open-marks", description or ""))
    if description is not None:
        checks.append(Check("removed-tests", "removed-tests", description, base))
    checks.extend(Check(f"python:{suite}", "python", suite) for suite in PYTHON_SUITES)
    return Plan(checks, notes, touched, users)


# --- Ворота PR ------------------------------------------------------------------

CI_WORKFLOW = ".github/workflows/ci.yml"
# Каждый шаг ворот PR и проверка гейта, которая его покрывает; `None` — служебный
# шаг, а не проверка. Шаг без имени записан первой строкой своей команды.
PR_GATES: Dict[str, Optional[str]] = {
    "Сохранить локальную сборку перед очисткой": None,
    "Вернуть локальную сборку в checkout": None,
    "python -m pip install -r tests/ci/requirements.txt": None,
    "Веха от владельца": None,
    "Зелёный итог этой вершины": None,
    "Статические проверки Python (запуск)": "python",
    "Статические проверки Python": "python",
    "Удалённые тесты (запуск)": "removed-tests",
    "Удалённые тесты объявлены": "removed-tests",
    "Формат": "fmt",
    "Контракт версий": "version-contract",
    "Clippy": "clippy",
    "Пометки незакрытого": "open-marks",
}
# Что из покрытого шага гейт на хосте не исполняет.
PARTIAL: Dict[str, str] = {
    "Удалённые тесты объявлены": "сверка с ночным инвентарём не выполняется: ей нужны артефакты прогонов GitHub, которых на этом хосте нет",
}
EVENT = re.compile(r"github\.event_name\s*==\s*'(\w+)'")


def events(condition: object) -> Set[str]:
    """События, при которых условие может пройти; без названного события — оба."""
    named = set(EVENT.findall(str(condition or "")))
    return named or {"pull_request", "merge_group"}


def step_name(step: Dict[str, object]) -> Optional[str]:
    if step.get("name"):
        return str(step["name"]).split(" #")[0].strip()
    run = str(step.get("run") or "").strip()
    return run.splitlines()[0].strip() if run else None


def gates(workflow: Dict[str, object]) -> Tuple[List[str], List[str]]:
    """Шаги с командой: (ворот PR, только очереди). Шаги `uses` — установка, не проверка."""
    pull: List[str] = []
    queue: List[str] = []
    for job in (workflow.get("jobs") or {}).values():
        job_events = events(job.get("if"))
        for step in job.get("steps") or []:
            name = step_name(step) if step.get("run") else None
            if not name:
                continue
            on = job_events & events(step.get("if"))
            if "pull_request" in on:
                pull.append(name)
            elif "merge_group" in on:
                queue.append(name)
    unique = lambda names: list(dict.fromkeys(names))  # noqa: E731
    return unique(pull), [name for name in unique(queue) if name not in pull]


def parity(root: Path, described: bool, dirty: bool = False) -> List[str]:
    """Строки отчёта о сверке с воротами PR."""
    try:
        import yaml  # noqa: PLC0415 — нужен только для сверки
        workflow = yaml.safe_load((root / CI_WORKFLOW).read_text(encoding="utf-8"))
    except (ImportError, OSError, ValueError) as error:
        return [f"Ворота PR не сверены: {CI_WORKFLOW} не прочитан ({error})"]
    pull, queue = gates(workflow)
    unknown = [name for name in pull if name not in PR_GATES]
    reasons: Dict[str, List[str]] = {name: [PARTIAL[name]] for name in pull if name in PARTIAL}
    for name in pull:
        if PR_GATES.get(name) == "removed-tests" and name in PARTIAL:
            if not described:
                reasons[name].insert(0, "описание PR не задано (`--description-file`), "
                                        "исчезнувшие тесты только напечатаны")
            if dirty:
                reasons[name].insert(0, "незакоммиченные правки не учтены: исчезнувшие тесты считаются по HEAD")
    if not described:
        for name in pull:
            if PR_GATES.get(name) == "open-marks":
                reasons.setdefault(name, []).append(
                    "описание PR не задано (`--description-file`): пометки на задачи, "
                    "которые закрывает этот PR, не сверены")
    unchecked = [f"{name} — {'; '.join(why)}" for name, why in reasons.items()]
    lines = [f"Ворота PR, которых гейт не знает: {'; '.join(unknown)} — сверьте PR_GATES "
             f"в scripts/dev/pre-ready-check.py с {CI_WORKFLOW}" if unknown
             else "Ворота PR: все шаги сверены с гейтом"]
    lines.append(f"Не проверено гейтом: {'; '.join(unchecked) if unchecked else 'нет'}")
    lines.append(f"Только в очереди, гейт не гоняет: {'; '.join(queue) if queue else 'нет'}")
    return lines


def removed_reminder(root: Path, base: str, env: Dict[str, str]) -> str:
    """Без описания PR исчезнувшие тесты — напоминание, а не отказ."""
    completed = subprocess.run(
        [sys.executable, "scripts/ci/removed_tests.py", "--base", base, "--description", ""],
        cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        encoding="utf-8", errors="replace")
    if completed.returncode not in (0, 1):
        detail = completed.stdout.strip().splitlines()
        return f"Исчезнувшие тесты не сверены: {detail[-1] if detail else f'код {completed.returncode}'}"
    lines = list(dict.fromkeys(REMOVED_DECLARE.findall(completed.stdout)))
    if not lines:
        return "Исчезнувшие тесты: нет"
    return "Исчезнувшие тесты — добавьте эти строки в описание PR:\n" + "\n".join(f"    {line}" for line in lines)


def load_cache(path: Path, base: str) -> Dict[str, List[str]]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    if data.get("version") != CACHE_VERSION or data.get("base") != base or data.get("platform") != sys.platform:
        return {}
    return {key: list(value) for key, value in data.get("checks", {}).items()}


def save_cache(path: Path, base: str, checks: Dict[str, List[str]]) -> None:
    """Пишет кеш целиком или не пишет: оборванная запись не оставляет полкеша."""
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_name(path.name + ".partial")
    partial.write_text(json.dumps({"version": CACHE_VERSION, "base": base, "platform": sys.platform,
                                   "checks": checks}, ensure_ascii=False, indent=1), encoding="utf-8")
    os.replace(partial, path)


def baseline(checks: Sequence[Check], base: str, cache_dir: Path,
             env: Dict[str, str]) -> Tuple[Dict[str, List[str]], List[str]]:
    """Падения тех же проверок на дереве базы и проверки, которые база не досчитала.

    Недостающие проверки досчитываются; в кеш попадает только досчитанная.
    """
    cache_path = cache_dir / "baseline.json"
    cached = load_cache(cache_path, base)
    missing = [check for check in checks if check.key not in cached]
    incomplete: List[str] = []
    if not missing:
        return cached, incomplete
    tree = cache_dir / "base"
    print(f"База {base[:12]}: считаю {len(missing)} проверок в {tree}", flush=True)
    add_base_tree(REPO_ROOT, tree, base)
    base_env = {**env, "CARGO_TARGET_DIR": str(cache_dir / "base-build")}
    try:
        for check in missing:
            # Исчезнувшее считается от базы: на самой базе сверять нечего. Пометки
            # сверяются с описанием этого PR и состоянием задач сейчас: тот же
            # отказ на базе не снимает его с PR, ворота откажут всё равно.
            if check.kind in BASELESS or (
                    check.kind == "test" and not crate_exists_on_base(tree, check.target)):
                cached[check.key] = []
                continue
            result = run_check(check, tree, base_env, cache_dir / "logs-base")
            # Авария сборки на базе — тоже недосчёт: закешированная как полный итог,
            # она вычла бы из ветки те же ошибки и спрятала бы настоящие.
            if not result.complete or result.unverified:
                incomplete.append(check.key)
                continue
            cached[check.key] = result.failures
            save_cache(cache_path, base, cached)
    finally:
        try:
            remove_base_tree(REPO_ROOT, tree)
        except Refusal as error:
            # Не заслонять итог проверок: копию уберёт следующий прогон.
            print(f"pre-ready-check: копия базы не убрана: {error}", file=sys.stderr)
    save_cache(cache_path, base, cached)
    return cached, incomplete


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


SYNTHETIC = ("сборка", "прогон оборвался", "набор упал", "форматирование:", "цель ")


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
    if not repeated.complete or repeated.unverified or any(name.startswith(SYNTHETIC)
                                                           for name in repeated.failures):
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


def log_path(log_dir: Path, key: str, suffix: str = ".log") -> Path:
    return log_dir / (re.sub(r"[^\w.-]+", "_", key) + suffix)


def repeat_after_crash(check: Check, root: Path, env: Dict[str, str], log_dir: Path) -> Tuple[Result, Path]:
    """Сборка не состоялась: один повтор, названный в итоге.

    Журнал первой попытки сохраняется: частота таких аварий — единственная
    улика об их причине, и молчаливый повтор её стёр бы.
    """
    first = log_path(log_dir, check.key, ".crash.log")
    os.replace(log_path(log_dir, check.key), first)
    print(f"  … сборка не состоялась, повтор {check.key} (журнал первой попытки: {first})", flush=True)
    return run_check(check, root, env, log_dir), first


def crash_report(unverified: Dict[str, List[str]], crash_logs: Dict[str, Path], log_dir: Path) -> List[str]:
    """Строки итога об авариях сборки: и не ушедших при повторе, и ушедших."""
    lines = [f"Авария сборки ушла при повторе: {key} — журнал первой попытки: {path}"
             for key, path in crash_logs.items() if key not in unverified]
    if unverified:
        keys = list(unverified)
        first = unverified[keys[0]][0]
        logs = "; ".join(f"{key}: {crash_logs.get(key, log_path(log_dir, key))}" for key in keys)
        repeated = " Повтор был, авария повторилась." if all(key in crash_logs for key in keys) else ""
        lines.append(f"Не проверено, сборка не состоялась: {', '.join(keys)} — тесты этих целей не исполнялись; "
                     f"первая ошибка: {first}; журнал: {logs}.{repeated} Повторите прогон; если повторится, "
                     "смотрите Clippy: он компилирует тот же код.")
    return lines


def check_env(environ: Mapping[str, str]) -> Dict[str, str]:
    """Окружение шагов гейта. Cargo собирает без инкремента, как CI
    (`dtolnay/rust-toolchain` выставляет `CARGO_INCREMENTAL=0`): иначе `debug/incremental`
    в каталоге исполнителя растёт с каждым прогоном, а ни гейт, ни CI его не читают.
    Явно заданное значение остаётся."""
    env = {**environ, "PYTHONUTF8": "1"}
    env.setdefault("CARGO_INCREMENTAL", "0")
    return env


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", default="origin/main", help="ветка базы, по умолчанию origin/main")
    parser.add_argument("--description-file", type=Path,
                        help="описание PR: исчезнувшие тесты сверяются с его строками Removes-test")
    args = parser.parse_args(argv)

    target = os.environ.get("CARGO_TARGET_DIR")
    if not target:
        print("pre-ready-check: задайте CARGO_TARGET_DIR — свой каталог сборки исполнителя", file=sys.stderr)
        return 2
    cache_dir = cargo_target_dir(REPO_ROOT).resolve() / CACHE_DIR_NAME
    env = check_env(os.environ)
    try:
        claim_target(cache_dir.parent, REPO_ROOT)
        base = git(REPO_ROOT, "merge-base", args.base, "HEAD").strip()
        lines = changed_lines(git(REPO_ROOT, "-c", "core.quotePath=false", "diff", "-U0", "--no-color", "--no-renames", base))
        dirty = git(REPO_ROOT, "status", "--porcelain", "--untracked-files=no").strip()
        packages = workspace_packages(REPO_ROOT, env)
    except Refusal as error:
        print(f"pre-ready-check: {error}", file=sys.stderr)
        return 2

    description = str(args.description_file.resolve()) if args.description_file else None
    whole = whole_build_reason(sorted(lines), base, {package.name for package in packages.values()})
    crates = sorted(packages) if whole else touched_crates(lines)
    planned = plan(crates, packages, base, description)
    checks = planned.checks
    if whole:
        # Выбор не по затронутым файлам: иначе длинный список стоял бы без объяснения.
        print(f"База {base[:12]} ({args.base}); крейты: весь workspace ({len(planned.touched)}) — "
              f"выбор по зависимостям, как у очереди: общая сборка: {whole}")
    else:
        print(f"База {base[:12]} ({args.base}); крейты: {', '.join(sorted(planned.touched)) or 'нет'}; "
              f"зависимые: {', '.join(sorted(planned.users)) or 'нет'}")
    if dirty:
        print("Рабочее дерево с незакоммиченными правками: итог относится к дереву, а не к HEAD.")
    for note in planned.notes:
        print(f"  {note}")

    current: Dict[str, Result] = {}
    crash_logs: Dict[str, Path] = {}
    try:
        for check in checks:
            current[check.key] = run_check(check, REPO_ROOT, env, cache_dir / "logs")
            if current[check.key].complete and current[check.key].unverified:
                current[check.key], crash_logs[check.key] = repeat_after_crash(check, REPO_ROOT, env,
                                                                               cache_dir / "logs")
    except KeyboardInterrupt:
        unrun = [check.key for check in checks if check.key not in current]
        print(f"\npre-ready-check: прогон прерван, итога нет. Не прогнаны: {', '.join(unrun)}", file=sys.stderr)
        return 130
    # Оборванную проверку ветки не с чем сверять: её итог неполон.
    unfinished = [check.key for check in checks if not current[check.key].complete]
    failed = [check for check in checks if current[check.key].failures and check.key not in unfinished]
    try:
        base_failures, base_unfinished = baseline(failed, base, cache_dir, env) if failed else ({}, [])
    except Refusal as error:
        print(f"pre-ready-check: база не посчитана: {error}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        print("\npre-ready-check: прогон прерван на базе, итога нет", file=sys.stderr)
        return 130
    for key in unfinished + base_unfinished:
        current.pop(key, None)
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

    unverified = {check.key: current[check.key].unverified for check in checks
                  if check.key in current and current[check.key].unverified}
    print()
    for check in checks:
        if check.key in unfinished or check.key in base_unfinished:
            where = "" if check.key in unfinished else " на базе"
            print(f"[не досчитано{where}] {check.key}")
            continue
        if check.key in unverified and not current[check.key].failures:
            print(f"[не проверено, сборка не состоялась] {check.key}")
            for line in unverified[check.key]:
                print(f"    {line}")
            continue
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
        for failure in unverified.get(check.key, []):
            print(f"    не проверено, сборка не состоялась: {failure}")
    crate_packages = {crate: package.name for crate, package in packages.items()}

    def read(name: str) -> Optional[str]:
        try:
            return (REPO_ROOT / name).read_text(encoding="utf-8")
        except OSError:
            return None

    unrun, elsewhere = unrun_tests(lines, read, current, crate_packages, host_facts())
    print()
    builds = sum(failure in build[key] for key, failures in fresh.items() for failure in failures)
    count = sum(map(len, fresh.values()))
    missing = unfinished + [key for key in base_unfinished if key not in unfinished]
    builds_note = f" (из них сбой сборки: {builds} — сборка продукта, а не проверка)" if builds else ""
    if missing or unverified:
        not_verified = f"; не проверено: {len(unverified)} целей" if unverified else ""
        print(f"Новые падения в досчитанных проверках: {count if fresh else 'нет'}{builds_note}{not_verified}")
    if missing:
        print(f"Итога нет: не досчитаны {', '.join(missing)}, поэтому о новых падениях судить нельзя. "
              "Повторите прогон.")
    elif not unverified:
        print(f"Новые падения: {count if fresh else 'нет'}{builds_note}")
    for line in crash_report(unverified, crash_logs, cache_dir / "logs"):
        print(line)
    print(f"Не исполнены на этой ОС, разобрать по коду: {'; '.join(unrun) if unrun else 'нет'}")
    print(f"Исполняются другим инструментом: {'; '.join(elsewhere) if elsewhere else 'нет'}")
    if description is None:
        print(removed_reminder(REPO_ROOT, base, env))
    for line in parity(REPO_ROOT, description is not None, bool(dirty)):
        print(line)
    print(f"Журналы: {cache_dir / 'logs'}")
    if missing or unverified:
        return INCOMPLETE_EXIT
    return 1 if fresh else 0


if __name__ == "__main__":
    raise SystemExit(main())
