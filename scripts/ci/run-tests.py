#!/usr/bin/env python3
"""Одна точка входа для прогона наборов CI обеих экосистем.

Workflow называет только этот скрипт — не `cargo test` и не `unittest`.
Профиль ворот меняет отбор в одном месте, а не в нескольких workflow.
Локальная агентная приёмка `tests/agent_evaluation` сюда не входит.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import allure_results  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
RUN_UNITTEST = Path(__file__).with_name("run-unittest.py")
PYTHON_SIZES = REPO_ROOT / ".config" / "python-sizes.toml"
NEXTEST_JUNIT = REPO_ROOT / "target" / "nextest" / "default" / "junit.xml"

from workflow_routing import (  # noqa: E402
    ADMITTED, LANED_SUITES, PROFILES, PYTHON_SUITES, SIZES,
    python_admitted, python_matrix, python_slug, python_suite_names,
)

ECOSYSTEMS = ("rust", "python", "all")


# Профиль ворот и профиль nextest — разные имена: ворота описывают, когда
# гоняем, профиль nextest — как. Ворота ложатся на одноимённые профили
# `.config/nextest.toml`, локальный `all` — на `default`.
NEXTEST_PROFILES = {
    "all": "default", "pr": "pr", "queue": "queue", "main": "main", "release": "release", "large": "nightly-full",
}


def nextest_profile(profile: str, selection: tuple[str, ...] | None = None) -> str:
    if selection == UREQ_RECYCLING_SELECTION:
        return "ureq-recycling"
    try:
        return NEXTEST_PROFILES[profile]
    except KeyError:
        raise ValueError(f"профиль {profile!r} для Rust не описан") from None


def nextest_junit(profile: str, selection: tuple[str, ...] | None = None) -> Path:
    """Nextest хранит JUnit относительно корня выбранного workspace."""
    workspace = REPO_ROOT / "vendor" / "ureq" if selection == UREQ_RECYCLING_SELECTION else REPO_ROOT
    return workspace / "target" / "nextest" / nextest_profile(profile, selection) / "junit.xml"


# Контракт ReceiptLedger (`tests/daemon_receipt_ledger.rs`) собирается только с
# признаком тестовой поддержки: у цели стоит `required-features`. Признак не
# включается на весь workspace — под ним рантайм ведёт себя иначе на пути
# fail-stop, а тестировать надо поставляемый код, — поэтому контракт идёт
# вторым вызовом nextest. Он целиком `medium` и `large`: ворота, принимающие
# только `small`, второго вызова не получают, потому что пустой отбор у
# nextest — отказ, а не зелёный ноль.
#
# Проверки обвязки сценариев идут в том же бинаре: их тела живут в библиотеке
# под признаком, а `#[test]` им приставляет контракт. Поэтому вызову не нужен
# `--lib` — тестовая сборка всего `unica-coder` ради них. Библиотека
# собирается один раз, обычной, для контракта и бинаря `unica`.
LEDGER_FEATURE_SELECTION = (
    "-p", "unica-coder", "--features", "receipt-ledger-test-support",
    "--test", "daemon_receipt_ledger",
    "-E", "binary(daemon_receipt_ledger)",
)

# A dependency's cfg(test) is not compiled by workspace tests. Its pinned
# Content-Length fault regressions use the same plan/result path, with no retry.
UREQ_RECYCLING_PREFIX = "completed_response_recycling::"
UREQ_RECYCLING_SELECTION = (
    "--manifest-path", "vendor/ureq/Cargo.toml", "--locked", "--lib",
    "--features", "json,serde_json/preserve_order,serde_json/float_roundtrip",
    "--config-file", "vendor/ureq/.config/nextest.toml", "--target-dir", "target",
    "-E", f"test(/^{UREQ_RECYCLING_PREFIX}/)",
)


# Вызов с признаком пересобирает `target/debug/unica` с другим набором
# признаков. Windows не даёт заменить файл, пока его исполняет живой процесс,
# а демоны тестов предыдущего вызова живут дольше теста, если держат
# обещанное состояние. Перед такой пересборкой на Windows печатаем выжившие
# `unica.exe` с командной строкой: она называет корень состояния демона,
# а по нему виден тест-источник. Процессы не завершаются и не ожидаются:
# утечку чинят в тесте, а здесь остаётся только свидетельство.
SURVIVING_PRODUCT_QUERY = (
    "powershell", "-NoProfile", "-NonInteractive", "-Command",
    "Get-CimInstance Win32_Process -Filter \"Name='unica.exe'\""
    " | Select-Object ProcessId,ParentProcessId,CommandLine | Format-List",
)


def rebuilds_product_with_features(selection: tuple[str, ...]) -> bool:
    return selection == LEDGER_FEATURE_SELECTION


def report_surviving_product_processes(run_query=subprocess.run) -> None:
    """Напечатать живые `unica.exe`; сбой самого запроса прогон не валит."""
    print("+ " + " ".join(SURVIVING_PRODUCT_QUERY), flush=True)
    try:
        run_query(list(SURVIVING_PRODUCT_QUERY), cwd=REPO_ROOT, check=False)
    except OSError as error:
        print(f"выжившие unica.exe не перечислены: {error}", flush=True)


def surviving_product_report(os_name: str = os.name):
    """Отчёт о выживших процессах нужен только там, где они держат файл."""
    return report_surviving_product_processes if os_name == "nt" else None


def rust_selections(profile: str) -> list[tuple[str, ...]]:
    """Что отбирает каждый вызов nextest: весь workspace и, если ворота
    принимают `medium` или `large`, одним вызовом контракт ReceiptLedger
    вместе с проверками его обвязки."""
    selections = [("--workspace",)]
    if set(ADMITTED[profile]) & {"medium", "large"}:
        selections.append(LEDGER_FEATURE_SELECTION)
        selections.append(UREQ_RECYCLING_SELECTION)
    return selections


def rust_command(profile: str, selection: tuple[str, ...]) -> list[str]:
    # nextest: процесс на тест и JUnit из коробки. Число потоков, повторы
    # и отчёт описаны в `.config/nextest.toml`, а не здесь: конвейер и
    # локальный прогон обязаны идти одной настройкой.
    return ["cargo", "nextest", "run", *selection, "--profile", nextest_profile(profile, selection)]


def rust_commands(profile: str) -> list[list[str]]:
    """Команды Rust для профиля, по одной на отбор."""
    return [rust_command(profile, selection) for selection in rust_selections(profile)]


def rust_list_commands(profile: str) -> list[list[str]]:
    """Состав тех же отборов: план обязан перечислять то, что прогон гоняет."""
    return [
        ["cargo", "nextest", "list", *selection, "--profile", nextest_profile(profile, selection),
         "--run-ignored", "all", "--message-format", "json"]
        for selection in rust_selections(profile)
    ]



def python_commands(
    profile: str,
    interpreter: str = sys.executable,
    results: Path | None = None,
    runner: str = "local",
    suite: str | None = None,
    only_size: str | None = None,
    docs_only: bool = False,
) -> list[list[str]]:
    """Команды Python для профиля: наборы тех размеров, что ворота принимают.

    Идут через `run-unittest.py`: это тот же `discover` и тот же текстовый
    вывод, но с классом результата, который пишет `allure-results`, когда
    указан каталог. Без каталога набор идёт как раньше и ничего не пишет.
    `suite` сужает прогон до одного набора: в CI наборы идут параллельными
    джобами, по одной на набор, и каждая зовёт этот же шов.
    """
    admitted = python_admitted(profile, docs_only=docs_only)
    if docs_only:
        if only_size not in (None, "small"):
            raise ValueError("docs-only исполняет только Python small")
        only_size = "small"
    if suite is not None and suite not in python_suite_names():
        raise ValueError(f"набор {suite!r} не описан; известны: {', '.join(python_suite_names())}")
    # Полоса размера: джоба гоняет один размер из допущенных воротами внутри
    # набора. Размер, которого ворота не принимают, даёт пустой список —
    # команд нет, а не «все». Сам набор допускается по его объявленному размеру.
    lane_sizes = list(admitted)
    if only_size:
        if only_size not in SIZES:
            raise ValueError(f"размер {only_size!r} не описан; известны: {', '.join(SIZES)}")
        lane_sizes = [size for size in admitted if size == only_size]
        if not lane_sizes:
            return []
    # Размер набора — умолчание; манифест поднимает отдельные классы и тесты
    # до `medium`. Ворота, допускающие не все размеры, получают `--admit`;
    # `all` без записи результатов повторяет прежнюю команду один в один.
    def tail(size: str) -> list[str]:
        sizing: list[str] = []
        if set(lane_sizes) != set(SIZES):
            sizing = ["--sizes", str(PYTHON_SIZES), "--admit", ",".join(lane_sizes)]
        if only_size:
            sizing += ["--lane", only_size]
        if docs_only and "medium" in ADMITTED[profile]:
            sizing += ["--scope-skip-medium"]
        if results is None:
            return (["--profile", profile] if docs_only else []) + ([] if size == "small" else ["--size", size]) + sizing
        return ["--results", str(results), "--runner", runner, "--profile", profile, "--size", size, "--sizes", str(PYTHON_SIZES), *sizing]

    return [
        [interpreter, str(RUN_UNITTEST), "-s", name, *extra, *tail(size)]
        for name, size, extra in PYTHON_SUITES
        if size in admitted and (suite is None or name == suite)
    ]


def commands(
    profile: str,
    ecosystem: str,
    interpreter: str = sys.executable,
    results: Path | None = None,
    runner: str = "local",
    suite: str | None = None,
    only_size: str | None = None,
    docs_only: bool = False,
) -> list[list[str]]:
    if profile not in PROFILES:
        raise ValueError(f"неизвестный профиль {profile!r}; известны: {', '.join(PROFILES)}")
    if ecosystem not in ECOSYSTEMS:
        raise ValueError(f"неизвестная экосистема {ecosystem!r}; известны: {', '.join(ECOSYSTEMS)}")
    if docs_only and ecosystem != "python":
        raise ValueError("docs-only допускается только для Python")
    planned: list[list[str]] = []
    if ecosystem in ("rust", "all"):
        planned.extend(rust_commands(profile))
    if ecosystem in ("python", "all"):
        planned.extend(python_commands(profile, interpreter, results, runner, suite, only_size, docs_only))
    return planned


def write_rust_plan(results: Path, profile: str) -> int:
    """План прогона — до тестов, чтобы упавший раннер не унёс его с собой."""
    entries = []
    for selection, command in zip(rust_selections(profile), rust_list_commands(profile), strict=True):
        cases = allure_results.nextest_list(REPO_ROOT, command)
        for case in cases:
            if rust_case_medium(selection, case["binary"], case["name"]):
                case["size"] = "medium"
        entries.extend(cases)
    allure_results.write_plan(results, entries)
    return len(entries)


def write_python_plan(profile: str, results: Path, runner: str, suite: str | None = None, only_size: str | None = None, docs_only: bool = False) -> int:
    """План Python — тем же швом и теми же командами, что и прогон, только состав."""
    for command in python_commands(profile, results=results, runner=runner, suite=suite, only_size=only_size, docs_only=docs_only):
        completed = subprocess.run([*command, "--plan-only"], cwd=REPO_ROOT, stdout=subprocess.DEVNULL)
        if completed.returncode != 0:
            raise SystemExit(f"план Python не записан: {' '.join(command)}")
    plan = results / "plan.json"
    entries = json.loads(plan.read_text(encoding="utf-8")) if plan.is_file() else []
    return sum(1 for entry in entries if entry.get("ecosystem") == "python")


def rust_case_medium(selection: tuple[str, ...] | None, binary: str, name: str) -> bool:
    return selection == UREQ_RECYCLING_SELECTION and binary == "ureq" and name.startswith(UREQ_RECYCLING_PREFIX)


def emit_rust(
    results: Path, profile: str, runner: str, junit: Path | None = None,
    *, selection: tuple[str, ...] | None = None,
) -> int:
    """JUnit от nextest + причины `#[ignore]` из атрибутов → allure-results."""
    junit = nextest_junit(profile, selection) if junit is None else junit
    reasons = allure_results.ignore_reasons(REPO_ROOT)
    entries = allure_results.junit_records(junit, runner=runner, profile=profile, reasons=reasons)
    for entry in entries:
        binary, _, name = entry["fullName"].partition("::")
        if rust_case_medium(selection, binary, name):
            for label in entry["labels"]:
                if label["name"] == "size":
                    label["value"] = "medium"
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
    parser.add_argument("--line", default=None, help="линия прогона для подписи; по умолчанию — из окружения")
    parser.add_argument("--sha", default=None, help="вершина линии для подписи; по умолчанию — из окружения")
    parser.add_argument("--suite", default=None, choices=python_suite_names(),
                        help="один набор Python вместо всех: в CI наборы идут джобами, по одной на набор")
    parser.add_argument("--only-size", default="", help="полоса размера набора Python; пусто — все допущенные")
    parser.add_argument("--python-matrix", action="store_true", help="напечатать матрицу джоб Python для ворот и выйти")
    parser.add_argument("--docs-only", nargs="?", const="true", default="false", choices=("true", "false"),
                        help="только Python small для подтверждённого изменения прозы")
    parser.add_argument("--phase", choices=("build", "run", "medium", "verify"),
                        help="this runner's lane archives, their execution, PR medium, or inventory verifier")
    parser.add_argument("--prepared", type=Path, help="job-local directory of the lane archives and manifests")
    parser.add_argument("--runners", default="[]", help="JSON runner matrix for complete inventory verification")
    parser.add_argument("--shadow", type=Path, help="shadow decision; mandatory selection remains full")
    parser.add_argument("--baseline", type=Path, help="previous Rust inventory of the same configuration")
    args = parser.parse_args(argv)
    args.docs_only = args.docs_only == "true"
    only_size = args.only_size or None

    if args.phase:
        import ci_rust_lanes
        if args.phase == "verify":
            if args.results is None or args.sha is None:
                parser.error("--phase verify requires --results and --sha")
            print(json.dumps(ci_rust_lanes.verify_union(args.results, args.profile, json.loads(args.runners), args.sha,
                                                         ci_rust_lanes.read_baseline(args.baseline)), indent=2))
            return 0
        if args.prepared is None or args.results is None:
            parser.error("--phase build/run/medium requires --prepared and --results")
        # Every lane of this runner, in the job that runs it: archives stay in
        # `--prepared`, plans and results go to the report directory.
        if args.phase == "build":
            return ci_rust_lanes.build_host(REPO_ROOT, args.prepared, args.results, args.profile, args.runner, args.line)
        if args.phase == "medium":
            return ci_rust_lanes.run_medium(REPO_ROOT, args.prepared / "ordinary", args.results / "ci-rust-medium", args.runner, args.shadow)
        return ci_rust_lanes.run_host(REPO_ROOT, args.prepared, args.results, args.profile, args.runner, args.line, args.shadow)

    if args.python_matrix:
        print(json.dumps(python_matrix(args.profile, docs_only=args.docs_only), ensure_ascii=False, separators=(",", ":")))
        return 0

    planned = commands(args.profile, args.ecosystem, results=args.results, runner=args.runner, suite=args.suite, only_size=only_size, docs_only=args.docs_only)
    if args.dry_run:
        for command in planned:
            print(" ".join(command))
        return 0

    if args.plan_only:
        if args.results is None:
            parser.error("--plan-only требует --results")
        # План едет с подписью: сайт сопоставляет его с результатами по линии
        # и раннеру, а не по имени артефакта.
        allure_results.write_run(
            args.results, profile=args.profile, runner=args.runner, ecosystem=args.ecosystem, line=args.line, sha=args.sha
        )
        if args.ecosystem in ("rust", "all"):
            print(f"план Rust: {write_rust_plan(args.results, args.profile)} тестов")
        if args.ecosystem in ("python", "all"):
            print(f"план Python: {write_python_plan(args.profile, args.results, args.runner, args.suite, only_size, args.docs_only)} тестов")
        return 0

    return execute(args.profile, args.ecosystem, args.results, args.runner, line=args.line, sha=args.sha, suite=args.suite, only_size=only_size, docs_only=args.docs_only)


def execute(
    profile: str,
    ecosystem: str,
    results: Path | None,
    runner: str,
    run_commands=None,
    junit: Path | None = None,
    line: str | None = None,
    sha: str | None = None,
    suite: str | None = None,
    only_size: str | None = None,
    before_feature_build=None,
    docs_only: bool = False,
) -> int:
    """Прогнать экосистемы и оставить результаты.

    Подпись прогона пишется один раз на вызов и до тестов: она описывает
    вызов, а не исход, и обязана остаться даже когда nextest упал раньше, чем
    успел написать JUnit. Результаты Rust пишутся, только если JUnit есть.
    """
    run_commands = run if run_commands is None else run_commands
    if docs_only:
        commands(profile, ecosystem, suite=suite, only_size=only_size, docs_only=True)
    if before_feature_build is None and run_commands is run:
        before_feature_build = surviving_product_report()
    if results is not None:
        allure_results.write_run(results, profile=profile, runner=runner, ecosystem=ecosystem, line=line, sha=sha)
    code = 0
    if ecosystem in ("rust", "all"):
        # Все вызовы nextest пишут JUnit в один каталог профиля, поэтому
        # результаты снимаются после каждого. Старый JUnit от прошлого
        # прогона — не результат этого: если nextest упадёт до отчёта, файл на
        # месте выдал бы чужие записи за свежие. Красный первый вызов второго
        # не отменяет — упавший тест не прячет остальных, — но Python после
        # красного Rust не идёт, как не шёл следующий шаг workflow.
        for selection, command in zip(rust_selections(profile), rust_commands(profile), strict=True):
            current_junit = nextest_junit(profile, selection) if junit is None else junit
            if current_junit.is_file():
                current_junit.unlink()
            if before_feature_build is not None and rebuilds_product_with_features(selection):
                before_feature_build()
            rust_code = run_commands([command])
            if results is not None and current_junit.is_file():
                print(f"результаты Rust: {emit_rust(results, profile, runner, current_junit, selection=selection)} записей")
            code = code or rust_code
        if code != 0:
            return code
    if ecosystem in ("python", "all"):
        code = run_commands(python_commands(profile, results=results, runner=runner, suite=suite, only_size=only_size, docs_only=docs_only))
    return code


if __name__ == "__main__":
    sys.exit(main())
