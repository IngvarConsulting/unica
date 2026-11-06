#!/usr/bin/env python3
"""Выбрать и прогнать тесты Rust записи очереди.

Всегда идёт весь `small`: вместе с Clippy всего workspace он ловит поломку интерфейса
в зависимых. К нему добавляется `medium` изменённых крейтов. Крейтом считается и файл
вне крейтов, который читают его тесты: `plugins`, `tests/fixtures`, `scripts` кроме
`scripts/ci`, по упоминанию пути в исходниках крейта. Полный набор идёт при правке
`Cargo.lock`, корневого `Cargo.toml`, тулчейна, при сбое чтения diff и при
`QUEUE_FULL_TESTS=true`. Остальное исполняет ночь; правило для красной ночи и откат —
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
from typing import Callable

ROOT = Path(__file__).resolve().parents[2]
# Меняют сборку всех крейтов: что затронуто, по пути не определить.
WHOLE_BUILD = re.compile(r"^(Cargo\.lock|Cargo\.toml|rust-toolchain[^/]*|\.cargo/.*)$")
# Читаются тестами крейтов, поэтому идут по упоминанию. Манифест сценариев читает только Python.
MENTIONED = re.compile(r"^(plugins/|tests/fixtures/(?!acceptance/scenario-manifest/)|scripts/(?!ci/))")


@dataclass(frozen=True)
class Choice:
    full: bool
    packages: tuple[str, ...]
    reason: str


def workspace(root: Path = ROOT) -> dict[str, str]:
    """Каталоги крейтов."""
    meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"], cwd=root, text=True, encoding="utf-8"))
    base = meta["workspace_root"].replace("\\", "/").rstrip("/")
    return {os.path.dirname(package["manifest_path"]).replace("\\", "/").removeprefix(base + "/"): package["name"] for package in meta["packages"]}


def crate_of(path: str, directories: dict[str, str]) -> str | None:
    best = max((name for name in directories if name and path.startswith(name + "/")), key=len, default=None)
    return directories.get(best) if best else None


def needle(path: str) -> str:
    """Что искать в исходниках: скрипт целиком, каталог набора данных или плагин."""
    parts = path.split("/")
    if parts[0] == "plugins":
        return "/".join(parts[:2])
    if parts[0] == "tests":
        return "/".join(parts[:3])
    return path


def mentions(text: str, root: Path = ROOT) -> list[str]:
    completed = subprocess.run(["git", "grep", "-lF", text, "--", "crates", "vendor"], cwd=root, capture_output=True, text=True, encoding="utf-8")
    if completed.returncode not in (0, 1):
        raise OSError(completed.stderr)
    return completed.stdout.split()


def choose(paths: list[str], directories: dict[str, str], *, force_full: bool = False, find: Callable[[str], list[str]] = mentions) -> Choice:
    if force_full:
        return Choice(True, (), "QUEUE_FULL_TESTS")
    if not paths:
        return Choice(True, (), "пустой diff")
    selected = set()
    for path in paths:
        if WHOLE_BUILD.match(path):
            return Choice(True, (), f"общая сборка: {path}")
        crate = crate_of(path, directories)
        if crate:
            selected.add(crate)
        elif MENTIONED.match(path):
            selected.update(crate for crate in (crate_of(item, directories) for item in find(needle(path))) if crate)
    return Choice(False, tuple(sorted(selected)), f"medium: {sorted(selected) or 'нет'}")


def load_runner():
    spec = importlib.util.spec_from_file_location("run_tests", ROOT / "scripts" / "ci" / "run-tests.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules["run_tests"] = module
    spec.loader.exec_module(module)
    return module


def commands(packages: tuple[str, ...], runner) -> list[list[str]]:
    """`small` остальных крейтов и `small` с `medium` выбранных: без повтора одних и тех же тестов."""
    others = ("--workspace", *(item for package in packages for item in ("--exclude", package)))
    planned = [[*runner.rust_command("pr", others), "--no-tests=pass"]]
    if packages:
        selection = tuple(item for package in packages for item in ("-p", package))
        planned.append([*runner.rust_command("queue", selection), "--no-tests=pass"])
    if "unica-coder" in packages:
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
        choice = choose([path for path in paths if path], workspace(), force_full=os.environ.get("QUEUE_FULL_TESTS") == "true")
    except (subprocess.CalledProcessError, OSError, ValueError) as error:
        choice = Choice(True, (), f"выбор не определён: {error}")
    print(f"{'полный набор' if choice.full else 'small всех крейтов'}: {choice.reason}", flush=True)
    outputs = os.environ.get("GITHUB_OUTPUT")
    if outputs:
        with open(outputs, "a", encoding="utf-8") as stream:
            stream.write(f"full={str(choice.full).lower()}\n")
    if args.dry_run:
        return 0
    runner = load_runner()
    planned = runner.rust_commands("queue") if choice.full else commands(choice.packages, runner)
    code = 0
    for command in planned:
        print("+ " + " ".join(command), flush=True)
        code = subprocess.run(command, cwd=ROOT, check=False).returncode or code
    return code


if __name__ == "__main__":
    sys.exit(main())
