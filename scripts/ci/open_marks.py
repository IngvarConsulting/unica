#!/usr/bin/env python3
"""Пометки незакрытого с номером задачи: открытой на PR, ни одной на выпуске.

Пометка ставится, когда изменение оставляет работу на следующее: суженную
проверку, временное значение, обход. Номер задачи делает её находимой поиском,
а не памятью; без номера пометка — забытое, а не отложенное.

Пометка — слово `TODO` и сразу за ним скобка с номером задачи `#N`; пример — в
скилле `unica-development`. Здесь форма не пишется целиком, чтобы этот файл сам
не стал пометкой. Пометки ищутся в исходниках: файлы с расширениями из
`CODE_SUFFIXES`.
Голое слово `TODO` пометкой не является: так пишутся заготовки кода 1С, которые
Unica сама кладёт в модули, справка по диагностике BSL и фикстуры. Документация
(`.md`) не код, поэтому пример пометки в ней проверку не задевает.

На PR (`pr`) пометка должна называть открытую задачу, не pull request. Задача,
которую этот PR закрывает (`Closes #N` в описании), тоже не годится: при
влитии она закроется, а пометка останется с номером, ведущим в закрытое.

На выпуске (`release`) пометок нет ни одной: кандидат и исправительная версия
не выходят, пока незакрытое не решено.

    python scripts/ci/open_marks.py pr
    python scripts/ci/open_marks.py release --rev <sha>
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable, Optional

ROOT = Path(__file__).resolve().parents[2]
CODE_SUFFIXES = (".rs", ".py", ".sh", ".ps1", ".yml", ".yaml", ".toml", ".js", ".ts", ".bsl")
# Слово и скобка разделены, чтобы этот файл сам не нёс пометки.
OPENER = "TODO" + "("
GREP_PATTERN = r"\bTODO\("
MARK = re.compile(r"\bTODO\(([^)\n]*)\)?")
NUMBER = re.compile(r"#([1-9]\d*)")
# Ключевые слова GitHub, закрывающие задачу при влитии PR: ссылка `#N`,
# `владелец/репозиторий#N` или адрес задачи. Чужой репозиторий не в счёт.
CLOSING = re.compile(
    r"\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s*:?\s+"
    r"(?:https://github\.com/(?P<url>[\w.-]+/[\w.-]+)/issues/|(?P<ref>[\w.-]+/[\w.-]+)?#)(?P<number>[1-9]\d*)\b",
    re.I,
)
FORM = f"`{OPENER}#<номер задачи>): <что снять и при каком условии>`"

OPEN, CLOSED, PULL, MISSING = "open", "closed", "pull", "missing"


@dataclass(frozen=True)
class Mark:
    path: str
    line: int
    text: str
    number: Optional[int]

    @property
    def place(self) -> str:
        return f"{self.path}:{self.line}"


def parse(path: str, line: int, text: str) -> list[Mark]:
    found = []
    for match in MARK.finditer(text):
        number = NUMBER.fullmatch(match.group(1).strip()) if match.group(0).endswith(")") else None
        found.append(Mark(path, line, text.strip(), int(number.group(1)) if number else None))
    return found


def grep(root: Path, rev: Optional[str]) -> str:
    """Строки с пометками в исходниках ревизии `rev` (или рабочего дерева) в виде `путь:строка:текст`."""
    command = ["git", "grep", "-n", "-I", "--no-color", "-E", GREP_PATTERN]
    if rev:
        command.append(rev)
    command += ["--", *(f"*{suffix}" for suffix in CODE_SUFFIXES)]
    completed = subprocess.run(command, cwd=root, capture_output=True, text=True, encoding="utf-8", check=False)
    if completed.returncode == 1 and not completed.stdout:
        return ""  # совпадений нет
    if completed.returncode != 0:
        raise RuntimeError(f"git grep завершился с кодом {completed.returncode}: {completed.stderr.strip()}")
    prefix = f"{rev}:" if rev else ""
    return "".join(line.removeprefix(prefix) + "\n" for line in completed.stdout.splitlines())


def marks(root: Path, rev: Optional[str] = None) -> list[Mark]:
    found = []
    for row in grep(root, rev).splitlines():
        path, line, text = row.split(":", 2)
        found += parse(path, int(line), text)
    return found


def closing_numbers(description: str, repository: str) -> set[int]:
    numbers = set()
    for match in CLOSING.finditer(description or ""):
        named = match.group("url") or match.group("ref")
        if named is None or named.lower() == repository.lower():
            numbers.add(int(match.group("number")))
    return numbers


def pr_problems(found: Iterable[Mark], state: Callable[[int], str], closing: set[int]) -> list[str]:
    problems = []
    for mark in found:
        if mark.number is None:
            problems.append(f"Пометка без номера задачи: {mark.place} — {mark.text}. Пометка пишется так: {FORM}.")
        elif mark.number in closing:
            problems.append(f"Этот PR закрывает #{mark.number}, а пометка на неё остаётся: {mark.place}. "
                            "Снимите пометку в этом же PR.")
        else:
            kind = state(mark.number)
            if kind == CLOSED:
                problems.append(f"Пометка ссылается на закрытую задачу #{mark.number}: {mark.place}. "
                                "Снимите пометку или откройте задачу заново.")
            elif kind == PULL:
                problems.append(f"Пометка ссылается на #{mark.number}, а это pull request, не задача: {mark.place}.")
            elif kind == MISSING:
                problems.append(f"Пометка ссылается на #{mark.number}, а такой задачи нет: {mark.place}.")
    return problems


CANDIDATE = ("Кандидат", "срезе", "до кандидата")
PATCH = ("Исправительная версия", "голове линии", "до выпуска")


def release_refusal(found: list[Mark], sha: str, release: tuple[str, str, str] = CANDIDATE) -> Optional[str]:
    """Отказ выпуска, пока в ревизии есть пометки; `None`, когда их нет."""
    if not found:
        return None
    subject, place, deadline = release
    listed = "; ".join(f"{mark.place} — #{mark.number}" if mark.number else f"{mark.place} — без номера"
                       for mark in found)
    return (f"{subject} не выпускается: в {place} {sha[:12]} остались пометки незакрытого ({len(found)}): "
            f"{listed}. Каждую решите {deadline}: доделайте работу или снимите пометку.")


def issue_state(repository: str, number: int) -> str:
    completed = subprocess.run(["gh", "api", f"repos/{repository}/issues/{number}"],
                               capture_output=True, text=True, encoding="utf-8", check=False)
    if completed.returncode != 0:
        if "404" in completed.stderr or "Not Found" in completed.stderr:
            return MISSING
        raise RuntimeError(f"gh api repos/{repository}/issues/{number}: {completed.stderr.strip()}")
    issue = json.loads(completed.stdout)
    if "pull_request" in issue:
        return PULL
    return OPEN if issue.get("state") == "open" else CLOSED


def event_description(repository: str) -> str:
    """Описание PR, прочитанное заново: правка после события тоже считается."""
    path = os.environ.get("GITHUB_EVENT_PATH")
    if os.environ.get("GITHUB_EVENT_NAME") != "pull_request" or not path:
        return ""
    number = json.loads(Path(path).read_text(encoding="utf-8"))["pull_request"]["number"]
    completed = subprocess.run(["gh", "api", f"repos/{repository}/pulls/{number}", "--jq", '.body // ""'],
                               capture_output=True, text=True, encoding="utf-8", check=True)
    return completed.stdout


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "IngvarConsulting/unica"))
    commands = parser.add_subparsers(dest="command", required=True)
    pr = commands.add_parser("pr", help="пометки называют открытые задачи, которые этот PR не закрывает")
    pr.add_argument("--description", help="описание PR; по умолчанию читается у GitHub")
    release = commands.add_parser("release", help="в ревизии нет ни одной пометки")
    release.add_argument("--rev", required=True)
    args = parser.parse_args(argv)
    if args.command == "release":
        refusal = release_refusal(marks(args.root, args.rev), args.rev)
        if refusal:
            print(f"::error::{refusal}", file=sys.stderr)
            return 1
        print("Пометок незакрытого в срезе нет.")
        return 0
    found = marks(args.root)
    description = args.description if args.description is not None else event_description(args.repo)
    problems = pr_problems(found, lambda number: issue_state(args.repo, number), closing_numbers(description, args.repo))
    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    if problems:
        return 1
    print(f"Пометок незакрытого: {len(found)}, все на открытые задачи.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
