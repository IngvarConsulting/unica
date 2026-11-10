#!/usr/bin/env python3
"""Права на вклад: реестр `CONTRIBUTORS.md` и почты коммитов PR.

Шаг «Права на вклад» в `.github/workflows/pr-metadata.yml` скачивает этот файл
из `main` и исполняет его над данными PR, прочитанными через API. Поэтому здесь
только стандартная библиотека и нет сети: на вход идут файлы.

- `--pr` — ответ `GET /repos/{repo}/pulls/{number}`;
- `--commits` — коммиты PR, по объекту JSON в строке;
- `--base` — `CONTRIBUTORS.md` в общей базе PR и `main`; файла может не быть;
- `--head` — `CONTRIBUTORS.md` в версии PR;
- `--holder` — логин правообладателя.

Каждая проблема — отдельная строка stdout. Код выхода 0 — проблем нет,
1 — проблемы найдены, 2 — входные данные не прочитаны.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

REGISTRY = "CONTRIBUTORS.md"
PARTICIPANTS = "Участники"
EXEMPT = "Без оферты"
COLUMNS = {
    PARTICIPANTS: ("GitHub", "ID", "Оферта", "PR", "Почты"),
    EXEMPT: ("Кто", "Почты", "Почему", "Допустимо в PR от"),
}
# Слово в «Допустимо в PR от»: PR от любого, у кого есть строка «Участники».
ANY_PARTICIPANT = "участники"
LOGIN = re.compile(r"@[A-Za-z0-9](?:[A-Za-z0-9]|-(?=[A-Za-z0-9])){0,38}")
AUTHOR = re.compile(r"@[A-Za-z0-9](?:[A-Za-z0-9]|-(?=[A-Za-z0-9])){0,38}(?:\[bot\])?")
ACCOUNT_ID = re.compile(r"[1-9][0-9]*")
VERSION = re.compile(r"[0-9]+(?:\.[0-9]+)*")
# `#0` — заготовка до открытия PR; изменённая строка с ней не проходит сверку номера.
PR_NUMBER = re.compile(r"#(?:0|[1-9][0-9]*)")
EMAIL = re.compile(r"[^\s@,<>|]+@[^\s@,<>|]+\.[^\s@,<>|]+")
SEPARATOR = re.compile(r":?-{3,}:?")
OFFER = re.compile(
    r"^\s*[-*]\s*\[[xX]\]\s*Я принимаю оферту Unica CLA версии\s+([0-9]+(?:\.[0-9]+)*)",
    re.MULTILINE,
)
CO_AUTHOR = re.compile(r"^\s*co-authored-by:.*<([^<>\s]+)>\s*$", re.IGNORECASE | re.MULTILINE)


@dataclass(frozen=True)
class Participant:
    line: int
    login: str
    account_id: str
    offer: str
    pr: str
    emails: tuple[str, ...]

    @property
    def key(self) -> str:
        return self.login[1:].lower()

    def content(self) -> tuple[str, ...]:
        return (self.login, self.account_id, self.offer, self.pr, *self.emails)


@dataclass(frozen=True)
class Exempt:
    line: int
    who: str
    emails: tuple[str, ...]
    why: str
    allowed: tuple[str, ...]

    def content(self) -> tuple[str, ...]:
        return (self.who, self.why, *self.emails, "|", *self.allowed)

    def admits(self, author: str, registry: Registry) -> bool:
        """Допустима ли почта строки в PR этого автора."""
        for token in self.allowed:
            if token == ANY_PARTICIPANT and registry.participant(author) is not None:
                return True
            if token.startswith("@") and token[1:] == author.lower():
                return True
        return False


@dataclass
class Registry:
    participants: list[Participant] = field(default_factory=list)
    exempt: list[Exempt] = field(default_factory=list)

    def owner_of(self, email: str) -> Participant | Exempt | None:
        for row in [*self.participants, *self.exempt]:
            if email in row.emails:
                return row
        return None

    def participant(self, login: str) -> Participant | None:
        return next((row for row in self.participants if row.key == login.lower()), None)


def cells(line: str) -> list[str] | None:
    text = line.strip()
    if not (text.startswith("|") and text.endswith("|") and len(text) > 1):
        return None
    return [cell.strip() for cell in text[1:-1].split("|")]


def emails_of(text: str) -> tuple[str, ...] | None:
    found = tuple(part.strip().lower() for part in text.split(","))
    if not found or not all(EMAIL.fullmatch(email) for email in found):
        return None
    return found


def allowed_of(text: str) -> tuple[str, ...] | None:
    found = tuple(part.strip() for part in text.split(","))
    if not found or not all(token == ANY_PARTICIPANT or AUTHOR.fullmatch(token) for token in found):
        return None
    return tuple(token.lower() for token in found)


def parse(text: str) -> tuple[Registry, list[str]]:
    """Реестр и проблемы формата; сломанная строка в реестр не попадает."""
    registry = Registry()
    problems: list[str] = []
    section: str | None = None
    seen: set[str] = set()
    table: list[int] = []  # номера строк таблицы текущего раздела
    seen_emails: dict[str, int] = {}
    seen_logins: dict[str, int] = {}

    def where(number: int) -> str:
        return f"{REGISTRY}, строка {number}"

    for number, line in enumerate(text.splitlines(), start=1):
        if line.startswith("## "):
            section = line[3:].strip()
            if section in seen:
                problems.append(f"{where(number)}: раздел «{section}» повторяется")
            seen.add(section)
            table = []
            continue
        if line.startswith("#"):
            section = None
            continue
        row = cells(line)
        if row is None:
            if line.strip().startswith("|"):
                problems.append(f"{where(number)}: строка таблицы должна начинаться и заканчиваться «|»")
            continue
        if section not in COLUMNS:
            problems.append(f"{where(number)}: таблица вне разделов «{PARTICIPANTS}» и «{EXEMPT}»")
            continue
        columns = COLUMNS[section]
        table.append(number)
        if len(table) == 1:
            if tuple(row) != columns:
                problems.append(f"{where(number)}: заголовок раздела «{section}» должен быть | {' | '.join(columns)} |")
            continue
        if len(table) == 2:
            if len(row) != len(columns) or not all(SEPARATOR.fullmatch(cell) for cell in row):
                problems.append(f"{where(number)}: после заголовка нужна строка-разделитель из {len(columns)} столбцов")
            continue
        if len(row) != len(columns):
            problems.append(f"{where(number)}: в разделе «{section}» нужно {len(columns)} столбцов, а их {len(row)}")
            continue
        entry: Participant | Exempt | None = None
        if section == PARTICIPANTS:
            login, account_id, offer, pr, emails = row
            parsed = emails_of(emails)
            errors = [
                message
                for ok, message in (
                    (LOGIN.fullmatch(login), f"логин «{login}» должен быть вида @login"),
                    (ACCOUNT_ID.fullmatch(account_id), f"ID «{account_id}» должен быть числом"),
                    (VERSION.fullmatch(offer), f"версия оферты «{offer}» должна быть вида 1.0"),
                    (PR_NUMBER.fullmatch(pr), f"PR «{pr}» должен быть вида #123"),
                    (parsed, f"почты «{emails}» должны идти через запятую"),
                )
                if not ok
            ]
            if not errors and login[1:].lower() in seen_logins:
                errors.append(f"{login} уже записан в строке {seen_logins[login[1:].lower()]}")
            if not errors:
                entry = Participant(number, login, account_id, offer, pr, parsed)
        else:
            who, emails, why, allowed = row
            parsed = emails_of(emails)
            admitted = allowed_of(allowed)
            errors = [
                message
                for ok, message in (
                    (who, "не указано, кто это"),
                    (parsed, f"почты «{emails}» должны идти через запятую"),
                    (why, "не указана причина"),
                    (admitted, f"«{allowed}»: перечислите через запятую @login или «{ANY_PARTICIPANT}»"),
                )
                if not ok
            ]
            if not errors:
                entry = Exempt(number, who, parsed, why, admitted)
        if entry is not None:
            for email in entry.emails:
                if email in seen_emails:
                    errors.append(f"почта {email} уже указана в строке {seen_emails[email]}")
        if errors:
            problems.extend(f"{where(number)}: {message}" for message in errors)
            continue
        for email in entry.emails:
            seen_emails[email] = number
        if isinstance(entry, Participant):
            seen_logins[entry.key] = number
            registry.participants.append(entry)
        else:
            registry.exempt.append(entry)

    for name in COLUMNS:
        if name not in seen:
            problems.append(f"{REGISTRY}: нет раздела «## {name}»")
    return registry, problems


def commit_emails(commit: dict) -> list[tuple[str, str, str | None]]:
    """(почта, роль, логин GitHub) автора и соавторов коммита."""
    details = commit.get("commit") or {}
    found: list[tuple[str, str, str | None]] = []
    author = ((details.get("author") or {}).get("email") or "").strip().lower()
    if author:
        found.append((author, "автора", (commit.get("author") or {}).get("login")))
    for email in CO_AUTHOR.findall(details.get("message") or ""):
        found.append((email.strip().lower(), "соавтора", None))
    return found


def check(pr: dict, commits: list[dict], base: str | None, head: str | None, holder: str) -> list[str]:
    if head is None:
        return [f"В PR нет файла {REGISTRY}"]
    registry, problems = parse(head)
    previous = parse(base)[0] if base is not None else Registry()
    author = (pr.get("user") or {}).get("login") or ""
    author_id = str((pr.get("user") or {}).get("id") or "")
    number = f"#{pr.get('number')}"
    body = pr.get("body") or ""

    reported: set[str] = set()
    for commit in commits:
        sha = (commit.get("sha") or "")[:7]
        for email, role, login in commit_emails(commit):
            row = registry.owner_of(email)
            if row is None:
                if email not in reported:
                    reported.add(email)
                    problems.append(
                        f"Коммит {sha}: почта {role} {email} не указана в {REGISTRY}. "
                        f"Примите оферту Unica CLA (CLA.md): добавьте свою строку с этой почтой "
                        f"в раздел «{PARTICIPANTS}» файла {REGISTRY} и отметьте пункт «Права на вклад» в описании PR"
                    )
                continue
            if isinstance(row, Exempt) and not row.admits(author, registry) and email not in reported:
                reported.add(email)
                problems.append(
                    f"Коммит {sha}: почта {role} {email} из таблицы «{EXEMPT}» допустима только в PR "
                    f"от {', '.join(row.allowed)}, а PR открыл @{author}"
                )
            named = row.login if isinstance(row, Participant) else row.who
            if login and named.startswith("@") and named[1:].lower() != login.lower():
                problems.append(
                    f"Коммит {sha}: GitHub связывает почту {email} с @{login}, "
                    f"а в {REGISTRY} она записана за {named} (строка {row.line})"
                )

    before = {row.key: row for row in previous.participants}
    after = {row.key: row for row in registry.participants}
    changed = sorted(key for key in before.keys() | after.keys()
                     if key not in before or key not in after or before[key].content() != after[key].content())
    offers = OFFER.findall(body)
    for key in changed:
        if key not in after:
            if author.lower() != holder.lower():
                problems.append(f"Строку {before[key].login} удалил PR от @{author}: удалять строки «{PARTICIPANTS}» может только @{holder}")
            continue
        row = after[key]
        if key != author.lower():
            problems.append(f"Строку {row.login} (строка {row.line}) меняет только её владелец, а PR открыл @{author}")
            continue
        if row.account_id != author_id:
            problems.append(f"Строка {row.line}: ID {row.account_id} не совпадает с ID аккаунта @{author} ({author_id})")
        if row.pr != number:
            problems.append(f"Строка {row.line}: в столбце PR {row.pr}, а это PR {number}")
        if not offers:
            problems.append("Отметьте в описании PR принятие оферты Unica CLA: блок «Права на вклад» шаблона PR")
        elif row.offer not in offers:
            problems.append(
                f"Строка {row.line}: оферта версии {row.offer}, а в описании PR отмечена версия {', '.join(offers)}"
            )

    exempt_before = sorted(row.content() for row in previous.exempt)
    exempt_after = sorted(row.content() for row in registry.exempt)
    if exempt_before != exempt_after and author.lower() != holder.lower():
        problems.append(f"Таблицу «{EXEMPT}» меняет только @{holder}, а PR открыл @{author}")
    return problems


def read_optional(path: Path) -> str | None:
    return path.read_text(encoding="utf-8") if path.is_file() else None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--pr", type=Path, required=True)
    parser.add_argument("--commits", type=Path, required=True)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--head", type=Path, required=True)
    parser.add_argument("--holder", required=True)
    args = parser.parse_args(argv)
    # `python3 -I` не читает PYTHONUTF8: кодировку вывода задаёт сам скрипт.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    try:
        pr = json.loads(args.pr.read_text(encoding="utf-8"))
        commits = [json.loads(line) for line in args.commits.read_text(encoding="utf-8").splitlines() if line.strip()]
    except (OSError, ValueError) as error:
        print(f"Данные PR не прочитаны: {error}")
        return 2
    problems = check(pr, commits, read_optional(args.base), read_optional(args.head), args.holder)
    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
