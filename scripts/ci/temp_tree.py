"""Удаление временного каталога теста вместе с демонами, которые его держат.

Демон Unica переживает процесс, который его поднял, держит файлы своего
состояния открытыми и, пока хранит квитанции, не выходит и по паузе простоя.
Windows не удаляет открытый файл и файл только для чтения. Поэтому перед
удалением останавливаются процессы, чья командная строка называет этот
каталог (демон с `--state-root` внутри и запущенные им движки), с файлов
снимается «только чтение», а само удаление повторяется до срока. Каталог, который так и не
удалился, — ошибка с именем каталога и причиной, а не тихий остаток в TEMP.
"""
from __future__ import annotations

import os
import shutil
import signal
import stat
import subprocess
import tempfile
import time
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path


def _processes() -> list[tuple[int, str]]:
    """(pid, командная строка) всех видимых процессов."""
    if os.name == "nt":
        # The daemon keeps its endpoint file open without read sharing, so the
        # process table, not the state directory, names its pid.
        script = "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId)`t$($_.CommandLine)\" }"
        listed = subprocess.run(["powershell", "-NoProfile", "-Command", script],
                                capture_output=True, check=False).stdout.decode("utf-8", "replace")
        found = []
        for line in listed.splitlines():
            pid, _, command = line.partition("\t")
            if pid.strip().isdigit() and command.strip():
                found.append((int(pid), command))
        return found
    found = []
    for entry in Path("/proc").glob("[0-9]*"):
        try:
            command = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode("utf-8", "replace")
        except OSError:
            continue
        if command.strip():
            found.append((int(entry.name), command))
    return found


def stop_holders(path: Path | str) -> list[int]:
    """Остановить процессы, чья командная строка называет каталог под `path`.

    Это демон с `--state-root` внутри и движки, которых он запустил на рабочей
    области теста. Свой процесс и родитель не трогаются.
    """
    root = os.path.normcase(str(Path(path).resolve()))
    own = {os.getpid(), os.getppid()}
    stopped = []
    for pid, command in _processes():
        if pid in own or root not in os.path.normcase(command):
            continue
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            continue
        stopped.append(pid)
    return stopped


def _writable_and_retry(function, path, _error) -> None:
    """Windows не удаляет файл только для чтения (`.git`, бинарь движка): снять атрибут и повторить."""
    os.chmod(path, stat.S_IWRITE)
    function(path)


def remove_tree(path: Path | str, timeout: float = 30.0) -> None:
    """Остановить держащие `path` процессы и удалить его, повторяя, пока они не выйдут."""
    stop_holders(path)
    deadline = time.monotonic() + timeout
    while True:
        try:
            shutil.rmtree(path, onexc=_writable_and_retry)
            return
        except FileNotFoundError:
            return
        except OSError as error:
            if time.monotonic() >= deadline:
                raise OSError(f"временный каталог {path} не удалён за {timeout:g} с: {error}") from error
            time.sleep(0.25)


@contextmanager
def temp_tree(prefix: str, timeout: float = 30.0) -> Iterator[str]:
    """`tempfile.TemporaryDirectory`, удаление которого ждёт выхода держащего процесса."""
    path = tempfile.mkdtemp(prefix=prefix)
    try:
        yield path
    finally:
        remove_tree(path, timeout)
