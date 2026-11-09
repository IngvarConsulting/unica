#!/usr/bin/env python3
"""Быстрые Python-проверки уровня PR: правила и контракты без сборки продукта.

Модули `tests/ci` и `tests/dev` идут параллельно, каждый в своём процессе.
Не идут `medium` и модули, объявившие `PR_SKIP` (`scripts/ci/python_sizes.py`):
их малым тестам нужен `cargo` или продукт, или они дольше бюджета. Всё это
исполняет очередь. Новый модуль попадает сюда сам; модуль дольше бюджета виден
предупреждением.
"""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "ci"))
import python_sizes  # noqa: E402

SUITES = ("tests/ci", "tests/dev")
WORKERS = 4
MODULE_BUDGET = 15.0


def skipped_modules(root: Path = ROOT) -> set[str]:
    return set(python_sizes.collect(root).pr_skip)


def kept_medium(root: Path = ROOT) -> list[str]:
    """`medium`, которые очередь оставляет себе: смоук продукта и снимки контрактов."""
    return list(python_sizes.collect(root).keep_medium)


def modules(root: Path = ROOT, skip: set[str] | None = None) -> list[str]:
    skip = skipped_modules() if skip is None else skip
    found = (f"{suite.replace('/', '.')}.{path.stem}" for suite in SUITES for path in sorted((root / suite).glob("test_*.py")))
    return [name for name in found if name not in skip]


def small_tests(suite: unittest.TestSuite, sizes, keep: tuple[str, ...] = ()) -> unittest.TestSuite:
    """Тесты модуля без `medium`: манифест называет их без префикса набора."""
    kept = unittest.TestSuite()
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            kept.addTest(small_tests(item, sizes, keep))
        else:
            name = item.id().split(".", 2)[-1]
            if sizes.size_of(name) == "small" or name.startswith(keep):
                kept.addTest(item)
    return kept


def run_module(name: str, keep: tuple[str, ...] = ()) -> int:
    """Рабочий процесс: один модуль, итог последней строкой в JSON."""
    spec = importlib.util.spec_from_file_location("run_unittest", ROOT / "scripts" / "ci" / "run-unittest.py")
    runner = importlib.util.module_from_spec(spec)
    sys.modules["run_unittest"] = runner
    spec.loader.exec_module(runner)
    # Как `discover`: каталог набора в пути, его помощники импортируются по имени.
    sys.path[:0] = [str(ROOT), str(ROOT / name.rsplit(".", 1)[0].replace(".", "/"))]
    suite = small_tests(unittest.defaultTestLoader.loadTestsFromName(name), runner.Sizes(list(python_sizes.collect(ROOT).medium)), keep)
    result = unittest.TextTestRunner(stream=sys.stdout, verbosity=0).run(suite)
    print(json.dumps({"tests": result.testsRun}))
    return 0 if result.wasSuccessful() else 1


def worker(name: str, queue: bool = False) -> tuple[str, int, float, str]:
    started = time.monotonic()
    completed = subprocess.run([sys.executable, __file__, "--module", name, *(["--queue"] if queue else [])], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, encoding="utf-8", errors="replace", env={**os.environ, "PYTHONUTF8": "1"})
    return name, completed.returncode, time.monotonic() - started, completed.stdout


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--module", help="рабочий режим: один модуль")
    parser.add_argument("--workers", type=int, default=WORKERS)
    parser.add_argument("--queue", action="store_true", help="уровень очереди: все модули, смоук и снимки контрактов")
    args = parser.parse_args(argv)
    if args.module:
        return run_module(args.module, tuple(kept_medium()) if args.queue else ())
    started = time.monotonic()
    with ThreadPoolExecutor(args.workers) as pool:
        results = list(pool.map(lambda name: worker(name, args.queue), modules(skip=set()) if args.queue else modules()))
    tests = 0
    failed = []
    for name, code, seconds, output in results:
        lines = output.rstrip().splitlines()
        if lines and lines[-1].startswith('{"tests"'):
            tests += json.loads(lines[-1])["tests"]
        if code != 0:
            failed.append(name)
            print(f"::group::{name}\n{output}\n::endgroup::")
        if seconds > MODULE_BUDGET and not args.queue:
            print(f"::warning::{name}: {seconds:.0f} с, бюджет {MODULE_BUDGET:.0f} с; такому модулю место в PR_SKIP")
    for name, _, seconds, _ in sorted(results, key=lambda item: -item[2])[:5]:
        print(f"{seconds:6.1f} с {name}")
    print(f"модулей {len(results)}, тестов {tests}, {time.monotonic() - started:.0f} с")
    for name in failed:
        print(f"::error::{name}: тесты не проходят")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
