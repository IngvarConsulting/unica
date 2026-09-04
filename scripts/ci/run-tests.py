#!/usr/bin/env python3
"""Одна точка входа для прогона тестов обеих экосистем.

Workflow называет только этот скрипт — не `cargo test` и не `unittest`. Пока
профиль один, `all`, и он повторяет прежние команды один в один: тот же
набор, тот же порядок, тот же цвет гейта. Смысл шага не в отборе, а в том,
что место для отбора появилось: профиль ворот меняет одну функцию здесь, а
не шесть мест в YAML.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

PROFILES = ("all",)
ECOSYSTEMS = ("rust", "python", "all")

# Наборы Python идут в том же порядке, что шли шагами workflow: сначала стражи
# CI, потом реестр, потом инструменты разработчика. `--durations` там, где
# набор длинный и стоит видеть, кто тянет время. Набор `tests/arch` идёт без
# `-t .`: его модули не пакет, и верхний уровень ему не нужен.
PYTHON_SUITES = (
    ("tests/ci", ("--durations", "20")),
    ("tests/arch", ()),
    ("tests/dev", ("--durations", "20")),
)


def rust_commands(profile: str) -> list[list[str]]:
    """Команды Rust для профиля. Сегодня — весь набор, одним потоком."""
    if profile == "all":
        # nextest: процесс на тест и JUnit из коробки. Число потоков, повторы
        # и отчёт описаны в `.config/nextest.toml`, а не здесь: конвейер и
        # локальный прогон обязаны идти одной настройкой.
        return [["cargo", "nextest", "run", "--workspace", "--profile", "default"]]
    raise ValueError(f"профиль {profile!r} для Rust не описан")


def python_commands(profile: str, interpreter: str = sys.executable) -> list[list[str]]:
    """Команды Python для профиля. Сегодня — три набора целиком."""
    if profile == "all":
        return [
            [interpreter, "-m", "unittest", "discover", "-s", suite, *extra]
            for suite, extra in PYTHON_SUITES
        ]
    raise ValueError(f"профиль {profile!r} для Python не описан")


def commands(profile: str, ecosystem: str, interpreter: str = sys.executable) -> list[list[str]]:
    if profile not in PROFILES:
        raise ValueError(f"неизвестный профиль {profile!r}; известны: {', '.join(PROFILES)}")
    if ecosystem not in ECOSYSTEMS:
        raise ValueError(f"неизвестная экосистема {ecosystem!r}; известны: {', '.join(ECOSYSTEMS)}")
    planned: list[list[str]] = []
    if ecosystem in ("rust", "all"):
        planned.extend(rust_commands(profile))
    if ecosystem in ("python", "all"):
        planned.extend(python_commands(profile, interpreter))
    return planned


def run(planned: list[list[str]]) -> int:
    """Выполнить команды по очереди; первая упавшая останавливает прогон.

    Так вели себя и шаги workflow: упавший шаг валит джобу, следующие не
    идут. Менять это здесь значило бы менять смысл гейта под видом переезда.
    """
    for command in planned:
        print("+ " + " ".join(command), flush=True)
        completed = subprocess.run(command, cwd=REPO_ROOT)
        if completed.returncode != 0:
            return completed.returncode
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--profile", required=True, choices=PROFILES, help="профиль ворот")
    parser.add_argument("--ecosystem", default="all", choices=ECOSYSTEMS)
    parser.add_argument("--dry-run", action="store_true", help="напечатать команды и выйти")
    args = parser.parse_args(argv)

    planned = commands(args.profile, args.ecosystem)
    if args.dry_run:
        for command in planned:
            print(" ".join(command))
        return 0
    return run(planned)


if __name__ == "__main__":
    sys.exit(main())
