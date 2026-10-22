#!/usr/bin/env python3
"""Запустить команду шага в фоне и позже дождаться её как отдельного шага.

`Unica CI` исполняет всё на одном раннере. Clippy упирается в компиляцию,
а прогон архива — в исполнение тестов; вместе они занимают раннер плотнее,
чем подряд. `start` запускает команду отсоединённо и сразу возвращается,
`wait` ждёт её конца, печатает её вывод и возвращает её код — так у шага
гейта остаются свой исход и свой лог. Нет записи о запуске — это отказ,
а не успех: шаг ожидания не должен позеленеть за пропущенный старт.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
STATE = ROOT / ".build" / "background"


def paths(name: str, state: Path = STATE) -> tuple[Path, Path, Path]:
    return state / f"{name}.started", state / f"{name}.log", state / f"{name}.exit"


def start(name: str, command: list[str], *, state: Path = STATE, cwd: Path = ROOT) -> int:
    started, log, exit_file = paths(name, state)
    state.mkdir(parents=True, exist_ok=True)
    for path in (started, log, exit_file):
        path.unlink(missing_ok=True)
    # Код пишется во временный файл и переименовывается: ожидающий не увидит
    # полузаписанный код.
    wrapper = ('"$@" > "$UNICA_BG_LOG" 2>&1; code=$?; '
               'printf "%s" "$code" > "$UNICA_BG_EXIT.tmp" && mv "$UNICA_BG_EXIT.tmp" "$UNICA_BG_EXIT"')
    environment = {**os.environ, "UNICA_BG_LOG": str(log), "UNICA_BG_EXIT": str(exit_file)}
    subprocess.Popen(["bash", "-c", wrapper, "background-step", *command], cwd=cwd, env=environment,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=True)
    started.write_text(" ".join(command) + "\n", encoding="utf-8")
    print(f"{name}: started in background: {' '.join(command)}", flush=True)
    return 0


def wait(name: str, *, state: Path = STATE, timeout: float, interval: float = 5,
         clock=time.monotonic, sleep=time.sleep) -> int:
    started, log, exit_file = paths(name, state)
    if not started.is_file():
        print(f"::error::{name}: no background start was recorded", flush=True)
        return 1
    deadline = clock() + timeout
    while not exit_file.is_file():
        if clock() >= deadline:
            print(f"::error::{name}: background command did not finish in {timeout:.0f} s", flush=True)
            if log.is_file():
                sys.stdout.write(log.read_text(encoding="utf-8", errors="replace"))
            return 1
        sleep(interval)
    if log.is_file():
        sys.stdout.write(log.read_text(encoding="utf-8", errors="replace"))
    code = exit_file.read_text(encoding="utf-8").strip()
    return int(code) if code.isdigit() else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="action", required=True)
    starting = sub.add_parser("start")
    starting.add_argument("name")
    starting.add_argument("command", nargs=argparse.REMAINDER)
    waiting = sub.add_parser("wait")
    waiting.add_argument("name")
    waiting.add_argument("--timeout", type=float, default=3600)
    args = parser.parse_args()
    if args.action == "start":
        command = args.command[1:] if args.command[:1] == ["--"] else args.command
        if not command:
            parser.error("start requires a command after --")
        return start(args.name, command)
    return wait(args.name, timeout=args.timeout)


if __name__ == "__main__":
    sys.exit(main())
