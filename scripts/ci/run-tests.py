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
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import allure_results  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
RUN_UNITTEST = Path(__file__).with_name("run-unittest.py")
NEXTEST_JUNIT = REPO_ROOT / "target" / "nextest" / "default" / "junit.xml"

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


# Профиль ворот и профиль nextest — разные имена: ворота описывают, когда
# гоняем, профиль nextest — как. Пока ворота одни, они ложатся на `default`.
NEXTEST_PROFILES = {"all": "default"}


def nextest_profile(profile: str) -> str:
    try:
        return NEXTEST_PROFILES[profile]
    except KeyError:
        raise ValueError(f"профиль {profile!r} для Rust не описан") from None


def rust_commands(profile: str) -> list[list[str]]:
    """Команды Rust для профиля. Сегодня — весь набор, одним потоком."""
    # nextest: процесс на тест и JUnit из коробки. Число потоков, повторы
    # и отчёт описаны в `.config/nextest.toml`, а не здесь: конвейер и
    # локальный прогон обязаны идти одной настройкой.
    return [["cargo", "nextest", "run", "--workspace", "--profile", nextest_profile(profile)]]


def python_commands(
    profile: str,
    interpreter: str = sys.executable,
    results: Path | None = None,
    runner: str = "local",
) -> list[list[str]]:
    """Команды Python для профиля. Сегодня — три набора целиком.

    Идут через `run-unittest.py`: это тот же `discover` и тот же текстовый
    вывод, но с классом результата, который пишет `allure-results`, когда
    указан каталог. Без каталога набор идёт как раньше и ничего не пишет.
    """
    if profile == "all":
        tail: list[str] = []
        if results is not None:
            tail = ["--results", str(results), "--runner", runner, "--profile", profile]
        return [
            [interpreter, str(RUN_UNITTEST), "-s", suite, *extra, *tail]
            for suite, extra in PYTHON_SUITES
        ]
    raise ValueError(f"профиль {profile!r} для Python не описан")


def commands(
    profile: str,
    ecosystem: str,
    interpreter: str = sys.executable,
    results: Path | None = None,
    runner: str = "local",
) -> list[list[str]]:
    if profile not in PROFILES:
        raise ValueError(f"неизвестный профиль {profile!r}; известны: {', '.join(PROFILES)}")
    if ecosystem not in ECOSYSTEMS:
        raise ValueError(f"неизвестная экосистема {ecosystem!r}; известны: {', '.join(ECOSYSTEMS)}")
    planned: list[list[str]] = []
    if ecosystem in ("rust", "all"):
        planned.extend(rust_commands(profile))
    if ecosystem in ("python", "all"):
        planned.extend(python_commands(profile, interpreter, results, runner))
    return planned


def write_rust_plan(results: Path, profile: str) -> int:
    """План прогона — до тестов, чтобы упавший раннер не унёс его с собой."""
    entries = allure_results.nextest_list(REPO_ROOT, nextest_profile(profile))
    allure_results.write_plan(results, entries)
    return len(entries)


def emit_rust(results: Path, profile: str, runner: str, junit: Path = NEXTEST_JUNIT) -> int:
    """JUnit от nextest + причины `#[ignore]` из атрибутов → allure-results."""
    reasons = allure_results.ignore_reasons(REPO_ROOT)
    entries = allure_results.junit_records(junit, runner=runner, profile=profile, reasons=reasons)
    for entry in entries:
        allure_results.write(results, entry)
    return len(entries)


def run(planned: list[list[str]]) -> int:
    """Выполнить команды по очереди; первая упавшая останавливает прогон.

    Так вели себя и шаги workflow: упавший шаг валит джобу, следующие не
    идут. Менять это здесь значило бы менять смысл гейта под видом переезда.
    Исключение одно: прогон nextest с упавшими тестами всё равно доводится до
    записи результатов — именно ради них он и шёл.
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
    parser.add_argument("--results", type=Path, default=None, help="куда писать allure-results")
    parser.add_argument("--runner", default=os.environ.get("RUNNER_NAME_LABEL", "local"),
                        help="имя раннера для меток и истории")
    parser.add_argument("--plan-only", action="store_true", help="записать план и выйти")
    args = parser.parse_args(argv)

    planned = commands(args.profile, args.ecosystem, results=args.results, runner=args.runner)
    if args.dry_run:
        for command in planned:
            print(" ".join(command))
        return 0

    if args.plan_only:
        if args.results is None:
            parser.error("--plan-only требует --results")
        if args.ecosystem in ("rust", "all"):
            print(f"план Rust: {write_rust_plan(args.results, args.profile)} тестов")
        return 0

    return execute(args.profile, args.ecosystem, args.results, args.runner)


def execute(
    profile: str,
    ecosystem: str,
    results: Path | None,
    runner: str,
    run_commands=None,
    junit: Path = NEXTEST_JUNIT,
) -> int:
    """Прогнать экосистемы и оставить результаты.

    Подпись прогона пишется один раз на вызов и до тестов: она описывает
    вызов, а не исход, и обязана остаться даже когда nextest упал раньше, чем
    успел написать JUnit. Результаты Rust пишутся, только если JUnit есть.
    """
    run_commands = run if run_commands is None else run_commands
    if results is not None:
        allure_results.write_run(results, profile=profile, runner=runner, ecosystem=ecosystem)
    code = 0
    if ecosystem in ("rust", "all"):
        code = run_commands(rust_commands(profile))
        if results is not None and junit.is_file():
            print(f"результаты Rust: {emit_rust(results, profile, runner, junit)} записей")
        if code != 0:
            return code
    if ecosystem in ("python", "all"):
        code = run_commands(python_commands(profile, results=results, runner=runner))
    return code


if __name__ == "__main__":
    sys.exit(main())
