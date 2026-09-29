#!/usr/bin/env python3
"""Дождаться, пока очередь слияния вольёт проверенное дерево в main.

Прогон очереди кончается раньше, чем очередь вливает его дерево: сайт,
собранный сразу, брал прежний `main` и отставал на одно вливание. Собирать
из самого коммита очереди тоже нельзя. Очередь пачечная, её прогоны
кончаются не по порядку, а pull request, снятый с очереди, всё равно даёт
успешный прогон. Коммит #1031 так и не лёг в `main`. Поэтому сайт ждёт, пока
`main` впитает проверенный коммит или очередь его отпустит, а собирается
из `main`.

Любой исход, включая отказ API, заканчивается кодом 0 и строкой в журнале.
Ожидание ограничено сроком, а сайт после него собирается из того `main`, что
есть. Шаг workflow ещё и не роняет джобу, если сам скрипт упадёт.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import time
from typing import Callable

# Статусы `compare/<коммит>...main`: коммит уже в `main`.
LANDED = {"identical", "ahead"}
# `main` ушёл в сторону от коммита: при вливании перемоткой он туда уже не
# ляжет, очередь пересоберёт свои элементы новыми коммитами.
STRANDED = "diverged"

MERGED = "влито"
REMOVED = "очередь отпустила дерево, не влив его"
DIVERGED = "main ушёл в сторону, дерево не ляжет"
TIMED_OUT = "не дождались вливания"


def holds(returncode: int, stdout: str, stderr: str, sha: str) -> bool:
    """Держит ли ветка очереди всё ещё проверенный коммит.

    404 — ветки нет. Другой коммит под тем же именем — очередь пересобрала
    элемент: у #1031 ветка `pr-1031-a9db66ec…` появлялась дважды. Прочие
    отказы API считаются «держит»: лучше подождать до срока, чем принять
    сбой за снятие с очереди.
    """
    if returncode == 0:
        return bool(sha) and stdout.strip().startswith(sha)
    return "HTTP 404" not in stderr


def wait(
    status: Callable[[], str | None],
    still_queued: Callable[[], bool],
    sleep: Callable[[float], None],
    clock: Callable[[], float],
    timeout: float,
    interval: float,
) -> str:
    """Опрашивать, пока коммит не ляжет в `main` или ждать станет нечего.

    `status` — статус сравнения коммита с `main` или `None`, если API не
    ответил. Очередь убирает ветку и после вливания, поэтому её пропажа
    проверяется только после сравнения, а перед выводом «отпустила»
    сравнение повторяется: вливание могло случиться между двумя запросами.
    """
    deadline = clock() + timeout
    while True:
        found = status()
        if found in LANDED:
            return MERGED
        if found == STRANDED:
            return DIVERGED
        if not still_queued():
            return MERGED if status() in LANDED else REMOVED
        if clock() >= deadline:
            return TIMED_OUT
        sleep(interval)


def gh(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["gh", "api", *args], capture_output=True, text=True)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", required=True)
    parser.add_argument("--sha", required=True, help="коммит, который проверила очередь")
    parser.add_argument("--queue-branch", required=True, help="ветка очереди gh-readonly-queue/...")
    parser.add_argument("--timeout", type=float, default=900, help="сколько секунд ждать")
    parser.add_argument("--interval", type=float, default=10, help="пауза между запросами, секунд")
    args = parser.parse_args(argv)

    def status() -> str | None:
        answer = gh(f"repos/{args.repo}/compare/{args.sha}...main", "--jq", ".status")
        return answer.stdout.strip() if answer.returncode == 0 else None

    def still_queued() -> bool:
        answer = gh(f"repos/{args.repo}/git/ref/heads/{args.queue_branch}", "--jq", ".object.sha")
        return holds(answer.returncode, answer.stdout, answer.stderr, args.sha)

    started = time.monotonic()
    outcome = wait(status, still_queued, time.sleep, time.monotonic, args.timeout, args.interval)
    print(f"{args.sha[:8]}: {outcome} за {time.monotonic() - started:.0f} с")
    return 0


if __name__ == "__main__":
    sys.exit(main())
