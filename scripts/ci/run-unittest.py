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
import sys
import traceback
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import allure_results  # noqa: E402


class AllureResult(unittest.TextTestResult):
    """Пишет запись в момент завершения теста; текстовый вывод не трогает."""

    out: Path | None = None
    runner_name = "local"
    profile = "all"
    suite_name = ""

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
    parser.add_argument("--plan-only", action="store_true", help="перечислить тесты и выйти")
    args = parser.parse_args(argv)

    suite = unittest.defaultTestLoader.discover(args.start_directory)
    if args.plan_only:
        for case in iter_cases(suite):
            print(case.id())
        return 0

    AllureResult.out = args.results
    AllureResult.runner_name = args.runner
    AllureResult.profile = args.profile
    AllureResult.suite_name = args.start_directory
    options = {"resultclass": AllureResult, "verbosity": 1}
    if args.durations is not None:
        options["durations"] = args.durations
    result = unittest.TextTestRunner(**options).run(suite)
    return 0 if result.wasSuccessful() else 1


def iter_cases(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from iter_cases(item)
        else:
            yield item


if __name__ == "__main__":
    sys.exit(main())
