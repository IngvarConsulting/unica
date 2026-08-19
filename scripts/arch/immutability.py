#!/usr/bin/env python3
"""Кто из продуктовых записей был отредактирован вместо замены.

Продуктовая запись — обещание, на которое кто-то снаружи уже опёрся. Дойдя до
целевой ветки, она становится историей и с этого дня говорит, что было решено на
её дату. Меняют не её, а преемника; сама она получает две строки: `superseded` и
имя заменившего.

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
# Замена — единственная законная правка принятой записи, и она трогает ровно эти
# два поля. Всё прочее означает, что запись переписали задним числом.
SUPERSESSION_FIELDS = ("status", "superseded-by")


@dataclass(frozen=True)
class Verdict:
    offenders: tuple[str, ...]
    compared: int

    def report(self) -> str:
        head = f"сверено продуктовых записей: {self.compared}"
        if not self.offenders:
            return head + "\nправок принятых записей нет"
        return head + "\n" + "\n".join(f"  {line}" for line in self.offenders)


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


def _is_supersession_only(before: str, after: str) -> bool:
    """Правка сводится к простановке замены и ничему больше."""
    old_props, old_body = _split(before)
    new_props, new_body = _split(after)
    if old_body != new_body:
        return False
    if set(old_props) != set(new_props):
        return False
    changed = [key for key in old_props if old_props[key] != new_props[key]]
    if not changed or any(key not in SUPERSESSION_FIELDS for key in changed):
        return False
    if "status" in changed and new_props["status"] != "superseded":
        return False
    if "superseded-by" in changed and not new_props["superseded-by"]:
        return False
    return True


def inspect(repo: Path, base_ref: str) -> Verdict:
    """Сверить принятые продуктовые записи базы с рабочим деревом."""
    base = _records_at(repo, base_ref)
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
        if after == before or _is_supersession_only(before, after):
            continue
        offenders.append(f"{path}: продуктовая запись отредактирована, а не заменена")

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
