#!/usr/bin/env python3
"""Выбрать и прогнать тесты Rust записи очереди.

Всегда идёт весь `small`: вместе с Clippy всего workspace он ловит поломку интерфейса
в зависимых. К нему добавляется `medium` изменённых крейтов. Крейтом считается и файл
вне крейтов, который читают его тесты: `tests/fixtures`, `scripts/dev`, по упоминанию пути
в исходниках крейта. Запись только из сайта, правил, навыков и прозы Rust не запускает:
её проверяют Python-наборы очереди. Прямые зависимые изменённого кода
исполняют интеграционные цели: логическая поломка доходит до них и без смены интерфейса.
`daemon_receipt_ledger` и транзитивные зависимые остаются ночи. Полный набор идёт при смене
внешних зависимостей в `Cargo.lock`, правке корневого `Cargo.toml`, тулчейна, при сбое чтения diff
и при `QUEUE_FULL_TESTS=true`. Правка `Cargo.lock` только в записях крейтов рабочей области полного
набора не требует: сами крейты выбраны по своим файлам. Остальное исполняет ночь; правило для красной ночи и откат —
в `.agents/skills/unica-testing/references/ci-pipeline.md`.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from typing import Callable

ROOT = Path(__file__).resolve().parents[2]
# Меняют сборку всех крейтов: что затронуто, по пути не определить.
WHOLE_BUILD = re.compile(r"^(Cargo\.lock|Cargo\.toml|rust-toolchain[^/]*|\.cargo/.*)$")
# Интеграционные цели прямых зависимых, контракт ReceiptLedger остаётся ночи.
DEPENDENT_FILTER = "kind(test) & not binary(daemon_receipt_ledger)"
# Читаются тестами крейтов, поэтому идут по упоминанию. Манифест сценариев читает только Python.
MENTIONED = re.compile(r"^(tests/fixtures/(?!acceptance/scenario-manifest/)|scripts/(?!ci/|research/))")
# Rust их не читает, а наборы `site`, стражи правил и навыков идут в Python очереди всегда.
NO_RUST = re.compile(r"^(docs/pages/|docs/visual-kit/|arch/rules/|\.agents/|scripts/research/|[^/]+\.md$)")
# Крейт, который собирает путь к данным из частей, читает любой их каталог.
ASSEMBLED_FIXTURES = '"tests/fixtures"'


@dataclass(frozen=True)
class Choice:
    full: bool
    packages: tuple[str, ...]
    reason: str
    dependents: tuple[str, ...] = ()
    no_rust: bool = False


def workspace(root: Path = ROOT) -> tuple[dict[str, str], dict[str, set[str]]]:
    """Каталоги крейтов и их прямые зависимые, включая dev-зависимости."""
    meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"], cwd=root, text=True, encoding="utf-8"))
    base = meta["workspace_root"].replace("\\", "/").rstrip("/")
    directories = {os.path.dirname(package["manifest_path"]).replace("\\", "/").removeprefix(base + "/"): package["name"] for package in meta["packages"]}
    names = set(directories.values())
    dependents: dict[str, set[str]] = {}
    for package in meta["packages"]:
        for dependency in package["dependencies"]:
            if dependency["name"] in names:
                dependents.setdefault(dependency["name"], set()).add(package["name"])
    return directories, dependents


def crate_of(path: str, directories: dict[str, str]) -> str | None:
    best = max((name for name in directories if name and path.startswith(name + "/")), key=len, default=None)
    return directories.get(best) if best else None


def needle(path: str) -> str:
    """Что искать в исходниках: скрипт целиком или каталог набора данных."""
    parts = path.split("/")
    if parts[0] == "tests":
        return "/".join(parts[:3])
    return path


def mentions(text: str, root: Path = ROOT) -> list[str]:
    completed = subprocess.run(["git", "grep", "-lF", text, "--", "crates", "vendor"], cwd=root, capture_output=True, text=True, encoding="utf-8")
    if completed.returncode not in (0, 1):
        raise OSError(completed.stderr)
    return completed.stdout.split()


def external_packages(lock: str) -> list[dict]:
    """Записи `Cargo.lock` о внешних пакетах: у крейта рабочей области нет `source`."""
    packages = tomllib.loads(lock).get("package", [])
    return sorted((package for package in packages if "source" in package), key=lambda package: (package["name"], package["version"], package["source"]))


def lock_keeps_external(base: str, head: str, root: Path = ROOT) -> bool:
    """Остались ли внешние зависимости прежними между двумя ревизиями."""
    locks = [subprocess.check_output(["git", "show", f"{revision}:Cargo.lock"], cwd=root, text=True, encoding="utf-8") for revision in (base, head)]
    return external_packages(locks[0]) == external_packages(locks[1])


def choose(paths: list[str], directories: dict[str, str], dependents: dict[str, set[str]] | None = None, *,
           force_full: bool = False, find: Callable[[str], list[str]] = mentions, lock_is_internal: bool = False) -> Choice:
    if force_full:
        return Choice(True, (), "QUEUE_FULL_TESTS")
    if not paths:
        return Choice(True, (), "пустой diff")
    if all(NO_RUST.match(path) for path in paths):
        return Choice(False, (), "Rust не нужен: сайт, правила, навыки, проза", no_rust=True)
    selected, edited = set(), set()
    for path in paths:
        if path == "Cargo.lock" and lock_is_internal:
            continue
        if WHOLE_BUILD.match(path):
            return Choice(True, (), f"общая сборка: {path}")
        crate = crate_of(path, directories)
        if crate:
            edited.add(crate)
            selected.add(crate)
        elif MENTIONED.match(path):
            texts = [needle(path), *([ASSEMBLED_FIXTURES] if path.startswith("tests/") else [])]
            selected.update(crate for text in texts for crate in (crate_of(item, directories) for item in find(text)) if crate)
    reached = sorted({name for crate in edited for name in (dependents or {}).get(crate, set())} - selected)
    return Choice(False, tuple(sorted(selected)), f"medium: {sorted(selected) or 'нет'}, цели зависимых: {reached or 'нет'}", tuple(reached))


def load_runner():
    spec = importlib.util.spec_from_file_location("run_tests", ROOT / "scripts" / "ci" / "run-tests.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules["run_tests"] = module
    spec.loader.exec_module(module)
    return module


def commands(packages: tuple[str, ...], runner, dependents: tuple[str, ...] = ()) -> list[list[str]]:
    """`small` остальных крейтов и `small` с `medium` выбранных: без повтора одних и тех же тестов."""
    others = ("--workspace", *(item for package in packages for item in ("--exclude", package)))
    planned = [[*runner.rust_command("pr", others), "--no-tests=pass"]]
    if packages:
        selection = tuple(item for package in packages for item in ("-p", package))
        planned.append([*runner.rust_command("queue", selection), "--no-tests=pass"])
    if dependents:
        selection = (*(item for package in dependents for item in ("-p", package)), "-E", DEPENDENT_FILTER)
        planned.append([*runner.rust_command("queue", selection), "--no-tests=pass"])
    if {"unica-coder", "unica-product"} & set(packages):
        planned.append(runner.rust_command("queue", runner.LEDGER_FEATURE_SELECTION))
    if "ureq" in packages:
        planned.append(runner.rust_command("queue", runner.UREQ_RECYCLING_SELECTION))
    return planned


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--dry-run", action="store_true", help="назвать выбор и не запускать")
    args = parser.parse_args(argv)
    try:
        paths = subprocess.check_output(["git", "diff", "--name-only", "--no-renames", args.base, args.head, "--"], cwd=ROOT, text=True, encoding="utf-8", errors="replace").split("\n")
        directories, dependents = workspace()
        paths = [path for path in paths if path]
        lock_is_internal = "Cargo.lock" in paths and lock_keeps_external(args.base, args.head)
        choice = choose(paths, directories, dependents, force_full=os.environ.get("QUEUE_FULL_TESTS") == "true", lock_is_internal=lock_is_internal)
    except (subprocess.CalledProcessError, OSError, ValueError) as error:
        choice = Choice(True, (), f"выбор не определён: {error}")
    print(f"{'полный набор' if choice.full else 'выбор'}: {choice.reason}", flush=True)
    outputs = os.environ.get("GITHUB_OUTPUT")
    if outputs:
        with open(outputs, "a", encoding="utf-8") as stream:
            stream.write(f"full={str(choice.full).lower()}\n")
    if args.dry_run:
        return 0
    runner = load_runner()
    if choice.no_rust:
        return 0
    planned = runner.rust_commands("queue") if choice.full else commands(choice.packages, runner, choice.dependents)
    code = 0
    for command in planned:
        print("+ " + " ".join(command), flush=True)
        code = subprocess.run(command, cwd=ROOT, check=False).returncode or code
    return code


if __name__ == "__main__":
    sys.exit(main())
