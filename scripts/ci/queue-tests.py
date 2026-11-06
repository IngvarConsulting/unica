#!/usr/bin/env python3
"""Выбрать и прогнать тесты Rust записи очереди.

Идут изменённые крейты и, если в их `src` менялась строка `pub`, крейты, прямо от них
зависящие. Поломку интерфейса в остальных ловят Clippy и компиляция всего workspace:
они идут всегда. Полный набор идёт, когда выбор неразрешим: общие файлы сборки,
манифесты, workflow, любой путь вне крейтов, кроме прозы и Python-тестов, сбой чтения
diff или `QUEUE_FULL_TESTS=true`. Пропущенное исполняет ночь; правило отката — в
`.agents/skills/unica-testing/references/ci-pipeline.md`.
"""

from __future__ import annotations

import argparse
import collections
from dataclasses import dataclass
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
PUB = re.compile(r"^[+-]\s*pub\s+(?!\()")
# Читаются сборкой или задают её: крейт по ним не определить.
UNRESOLVABLE = re.compile(r"(^|/)(Cargo\.toml|Cargo\.lock|build\.rs|rust-toolchain[^/]*)$|^\.(cargo|config|github)/|^scripts/")
# Вне крейтов и без влияния на Rust: тесты Rust и сборка эти пути не читают. Страж
# в `tests/ci/test_queue_tests.py` следит за этим. `.md` в крейте — правка крейта.
NO_RUST = re.compile(
    r"^(?!plugins/|tests/).*\.md$|^docs/|^\.agents/|^tests/(ci|dev)/|^tests/fixtures/acceptance/scenario-manifest/"
    r"|^\.config/python-sizes\.toml$|^\.github/(CODEOWNERS|ISSUE_TEMPLATE/|PULL_REQUEST_TEMPLATE)"
)


@dataclass(frozen=True)
class Choice:
    full: bool
    packages: tuple[str, ...]
    reason: str


def workspace(root: Path = ROOT) -> tuple[dict[str, str], dict[str, set[str]]]:
    """Каталоги крейтов и их прямые зависимые, включая dev-зависимости."""
    meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"], cwd=root, text=True, encoding="utf-8"))
    base = meta["workspace_root"].replace("\\", "/").rstrip("/")
    names = {package["name"] for package in meta["packages"]}
    directories, dependents = {}, collections.defaultdict(set)
    for package in meta["packages"]:
        directories[os.path.dirname(package["manifest_path"]).replace("\\", "/").removeprefix(base + "/")] = package["name"]
        for dependency in package["dependencies"]:
            if dependency["name"] in names:
                dependents[dependency["name"]].add(package["name"])
    return directories, dependents


def crate_of(path: str, directories: dict[str, str]) -> str | None:
    best = max((name for name in directories if name and path.startswith(name + "/")), key=len, default=None)
    return directories.get(best) if best else None


def parse_diff(text: str) -> dict[str, list[str]]:
    """Изменённые строки по файлам из `git diff --unified=0 --no-renames`."""
    files: dict[str, list[str]] = {}
    current: list[str] | None = None
    for line in text.splitlines():
        if line.startswith("diff --git "):
            current = None
        elif line.startswith(("--- a/", "+++ b/")):
            current = files.setdefault(line[6:], [])
        elif current is not None and line.startswith(("+", "-")) and not line.startswith(("+++", "---")):
            current.append(line)
    return files


def choose(files: dict[str, list[str]], directories: dict[str, str], dependents: dict[str, set[str]], *, force_full: bool = False) -> Choice:
    if force_full:
        return Choice(True, (), "QUEUE_FULL_TESTS")
    if not files:
        return Choice(True, (), "пустой diff")
    changed, public = set(), set()
    for path, lines in files.items():
        crate = crate_of(path, directories)
        if crate is None and NO_RUST.search(path):
            continue
        if UNRESOLVABLE.search(path):
            return Choice(True, (), f"неразрешимый путь {path}")
        if crate is None:
            return Choice(True, (), f"путь вне крейтов {path}")
        changed.add(crate)
        if "/src/" in path and any(PUB.match(line) for line in lines):
            public.add(crate)
    selected = set(changed)
    for crate in public:
        selected |= dependents.get(crate, set())
    return Choice(False, tuple(sorted(selected)), f"изменены {sorted(changed)}, строки pub в {sorted(public)}")


def load_runner():
    spec = importlib.util.spec_from_file_location("run_tests", ROOT / "scripts" / "ci" / "run-tests.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules["run_tests"] = module
    spec.loader.exec_module(module)
    return module


def commands(packages: tuple[str, ...], runner) -> list[list[str]]:
    """Те же вызовы nextest, что у полного набора, суженные до выбранных крейтов."""
    planned = []
    if packages:
        selection = tuple(item for package in packages for item in ("-p", package))
        planned.append(runner.rust_command("queue", selection))
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
        text = subprocess.check_output(["git", "diff", "--unified=0", "--no-renames", args.base, args.head, "--"], cwd=ROOT, text=True, encoding="utf-8", errors="replace")
        directories, dependents = workspace()
        choice = choose(parse_diff(text), directories, dependents, force_full=os.environ.get("QUEUE_FULL_TESTS") == "true")
    except (subprocess.CalledProcessError, OSError, ValueError) as error:
        choice = Choice(True, (), f"выбор не определён: {error}")
    print(f"{'полный набор' if choice.full else 'выбор ' + ' '.join(choice.packages)}: {choice.reason}", flush=True)
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
