#!/usr/bin/env python3
"""Прогон набора `unittest` с записью результатов в формате Allure.

Замена `python -m unittest discover` один в один: тот же поиск, тот же
текстовый вывод, тот же код выхода. Сверх того — свой класс результата,
который пишет `{uuid}-result.json` на каждый тест, когда указан `--results`.
Без него набор идёт как раньше и ничего не пишет.

JUnit здесь не нужен: `unittest` принимает свой класс результата, он в
процессе и знает про тест всё.
"""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
import traceback
import unittest
from pathlib import Path

# `python -m unittest` ставит текущий каталог первым в `sys.path`, и модули
# тестов импортируют `scripts.ci.*` от корня. Скрипт по умолчанию ставит
# свой каталог, а не корень, — восстанавливаем то, что было у `-m`.
REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import allure_results  # noqa: E402


class Sizes:
    """Размер теста по манифесту `.config/python-sizes.toml`.

    Запись манифеста — `модуль`, `модуль.Класс` или `модуль.Класс.тест`;
    побеждает самая длинная подходящая. Без записи размер — размер набора.
    """

    def __init__(self, medium: list[str], default: str = "small"):
        self.medium = sorted(medium, key=len, reverse=True)
        self.default = default

    @classmethod
    def load(cls, path: Path | None, default: str = "small") -> "Sizes":
        if path is None or not path.is_file():
            return cls([], default)
        config = tomllib.loads(path.read_text(encoding="utf-8"))
        return cls(list(config.get("medium", {}).get("entries", [])), default)

    def size_of(self, test_id: str) -> str:
        for entry in self.medium:
            if test_id == entry or test_id.startswith(entry + "."):
                return "medium"
        return self.default


class AllureResult(unittest.TextTestResult):
    """Пишет запись в момент завершения теста; текстовый вывод не трогает."""

    out: Path | None = None
    runner_name = "local"
    profile = "all"
    suite_name = ""
    sizes = Sizes([])

    def startTest(self, test):
        super().startTest(test)
        self._started = allure_results.now_ms()

    def _emit(self, test, status, message=None, trace=None):
        if self.out is None:
            return
        cls = test.__class__
        full_name = f"{cls.__module__}.{cls.__qualname__}.{test._testMethodName}"
        doc = (test._testMethodDoc or "").strip().splitlines()
        allure_results.write(
            self.out,
            allure_results.record(
                name=doc[0] if doc else test._testMethodName,
                full_name=full_name,
                status=status,
                runner=self.runner_name,
                labels={
                    "language": "python",
                    "framework": "unittest",
                    "parentSuite": "python",
                    "suite": self.suite_name,
                    "subSuite": cls.__qualname__,
                    "profile": self.profile,
                    "size": self.sizes.size_of(test.id()),
                },
                tags=(self.profile,),
                message=message,
                trace=trace,
                start=self._started,
                stop=allure_results.now_ms(),
            ),
        )

    def addSuccess(self, test):
        super().addSuccess(test)
        self._emit(test, "passed")

    def addFailure(self, test, err):
        super().addFailure(test, err)
        self._emit(test, "failed", str(err[1]), "".join(traceback.format_exception(*err)))

    def addError(self, test, err):
        super().addError(test, err)
        self._emit(test, "broken", f"{err[0].__name__}: {err[1]}", "".join(traceback.format_exception(*err)))

    def addSkip(self, test, reason):
        super().addSkip(test, reason)
        self._emit(test, "skipped", reason)

    def addExpectedFailure(self, test, err):
        super().addExpectedFailure(test, err)
        self._emit(test, "passed", "ожидаемое падение")

    def addUnexpectedSuccess(self, test):
        super().addUnexpectedSuccess(test)
        self._emit(test, "failed", "неожиданный успех")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("-s", "--start-directory", required=True)
    parser.add_argument("--durations", type=int, default=None)
    parser.add_argument("--results", type=Path, default=None, help="каталог allure-results")
    parser.add_argument("--runner", default="local")
    parser.add_argument("--profile", default="all")
    parser.add_argument("--size", default="small", help="размер набора: умолчание для тестов без записи в манифесте")
    parser.add_argument("--sizes", type=Path, default=None, help="манифест размеров .config/python-sizes.toml")
    parser.add_argument("--admit", default="", help="какие размеры гонять, через запятую; пусто — все")
    parser.add_argument("--plan-only", action="store_true", help="перечислить тесты и выйти")
    parser.add_argument("--lane", default="", help="полоса размера этой джобы: идёт в план, чтобы сайт нашёл её джобу")
    args = parser.parse_args(argv)

    sizes = Sizes.load(args.sizes, args.size)
    admitted = {size.strip() for size in args.admit.split(",") if size.strip()}
    suite = unittest.defaultTestLoader.discover(args.start_directory)
    # Не допущенный воротами тест — не в этом плане: его результат в отчёте
    # линии даёт другой профиль, а не пропуск от этого.
    if admitted:
        suite = unittest.TestSuite(case for case in iter_cases(suite) if sizes.size_of(case.id()) in admitted)
    if args.plan_only:
        planned = list(iter_cases(suite))
        for case in planned:
            print(case.id())
        if args.results is not None:
            write_python_plan(args.results, planned, args.start_directory, sizes, args.lane)
        return 0

    AllureResult.out = args.results
    AllureResult.runner_name = args.runner
    AllureResult.profile = args.profile
    AllureResult.suite_name = args.start_directory
    AllureResult.sizes = sizes
    options = {"resultclass": AllureResult, "verbosity": 1}
    if args.durations is not None:
        options["durations"] = args.durations
    result = unittest.TextTestRunner(**options).run(suite)
    return 0 if result.wasSuccessful() else 1


def write_python_plan(out: Path, cases, suite_name: str, sizes: Sizes, lane: str = "") -> Path:
    """Состав набора до запуска: джоба, умершая на середине, оставляет список.

    Записи идут с той же подписью, что и результаты; сайт дописывает
    недошедшие тесты `skipped`. Планы нескольких наборов в одном каталоге
    складываются, а не затирают друг друга; свой набор переписывается.
    """
    out.mkdir(parents=True, exist_ok=True)
    path = out / "plan.json"
    entries = json.loads(path.read_text(encoding="utf-8")) if path.is_file() else []
    entries = [
        entry for entry in entries
        if not (entry.get("ecosystem") == "python" and entry.get("suite") == suite_name and entry.get("lane", "") == lane)
    ]
    for case in cases:
        doc = (getattr(case, "_testMethodDoc", None) or "").strip().splitlines()
        entries.append(
            {
                "ecosystem": "python",
                "id": case.id(),
                "name": doc[0] if doc else getattr(case, "_testMethodName", case.id()),
                "suite": suite_name,
                "lane": lane,
                "subSuite": case.__class__.__qualname__,
                "size": sizes.size_of(case.id()),
            }
        )
    path.write_text(json.dumps(entries, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    return path


def iter_cases(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from iter_cases(item)
        else:
            yield item


if __name__ == "__main__":
    sys.exit(main())
