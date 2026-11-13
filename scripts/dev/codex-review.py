#!/usr/bin/env python3
"""Независимое ревью итогового diff через Codex CLI.

    python scripts/dev/codex-review.py                  # против origin/main
    python scripts/dev/codex-review.py --base main
    python scripts/dev/codex-review.py --commit <sha>

Скрипт находит `codex`, запускает `codex review` в корне рабочей копии,
складывает вывод в файл под `.session-temp/codex-review/` и печатает его путь.
Своего промпта скрипт не добавляет: без него Codex применяет собственные
инструкции ревью и сам читает `AGENTS.md` и записи `arch/rules`.

Песочницу скрипт не переключает: ревью ничего не пишет, а `--sandbox
workspace-write` на Windows зависает. Рабочая копия может быть worktree:
каталог `.git` тогда лежит в основном клоне, выше корня копии, и Codex
читает его из песочницы по умолчанию.

Код выхода 0, когда Codex дошёл до конца, даже если нашёл замечания:
их разбирает исполнитель. 1 — сбой самого Codex или его таймаут,
2 — отказ до запуска: Codex не найден, вход не выполнен, нет такой базы.
"""

from __future__ import annotations

import argparse
import os
import shutil
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Callable, Iterable, List, Optional, Sequence


REPO_ROOT = Path(__file__).resolve().parents[2]
OUT_DIR = Path(".session-temp") / "codex-review"
DEFAULT_BASE = "origin/main"
DEFAULT_TIMEOUT = 1800
# Установщик Codex кладёт бинарь в каталог с хешем сборки. Хеш меняется
# при обновлении, поэтому путь ищется шаблоном, а не запоминается.
INSTALL_GLOBS = {
    "win32": (
        "{LOCALAPPDATA}/OpenAI/Codex/bin/*/codex.exe",
        "{HOME}/.codex/bin/*/codex.exe",
    ),
    "darwin": (
        "{HOME}/Library/Application Support/OpenAI/Codex/bin/*/codex",
        "{HOME}/.codex/bin/*/codex",
    ),
    "other": (
        "{HOME}/.local/share/OpenAI/Codex/bin/*/codex",
        "{HOME}/.codex/bin/*/codex",
    ),
}
FALLBACK = "Ревью можно сделать субагентом: это запасной путь"


class Refusal(Exception):
    """Запускать Codex незачем: его нет, вход не выполнен или база не та."""


def install_globs(platform: str) -> Sequence[str]:
    return INSTALL_GLOBS.get(platform, INSTALL_GLOBS["other"])


def expand(pattern: str, environ: dict) -> Optional[str]:
    """Подставить домашний каталог и LOCALAPPDATA; None, если их нет."""
    home = environ.get("HOME") or environ.get("USERPROFILE")
    local = environ.get("LOCALAPPDATA")
    for key, value in (("{HOME}", home), ("{LOCALAPPDATA}", local)):
        if key in pattern:
            if not value:
                return None
            pattern = pattern.replace(key, value.replace("\\", "/"))
    return pattern


def glob_pattern(pattern: str) -> Iterable[Path]:
    path = Path(pattern)
    if not path.anchor:
        return Path(".").glob(pattern)
    root = Path(path.anchor)
    return root.glob(str(path.relative_to(root)).replace("\\", "/"))


def newest(paths: Iterable[Path]) -> Optional[Path]:
    """Свежайший по времени файла: обновление ставит бинарь в новый каталог."""
    known = [path for path in paths if path.is_file()]
    if not known:
        return None
    return max(known, key=lambda path: (path.stat().st_mtime, str(path)))


def find_codex(environ: dict, platform: str, which: Callable[[str], Optional[str]]) -> Path:
    explicit = environ.get("CODEX_BIN")
    if explicit:
        if not Path(explicit).is_file():
            raise Refusal(f"CODEX_BIN указывает на {explicit}, такого файла нет")
        return Path(explicit)
    found = which("codex")
    if found:
        return Path(found)
    candidates: List[Path] = []
    for pattern in install_globs(platform):
        expanded = expand(pattern, environ)
        if expanded is not None:
            candidates.extend(glob_pattern(expanded))
    best = newest(candidates)
    if best is None:
        raise Refusal(
            "Codex не найден: его нет ни в PATH, ни в каталоге установки "
            f"({', '.join(install_globs(platform))}). Поставьте Codex CLI "
            f"или задайте путь в CODEX_BIN. {FALLBACK}"
        )
    return best


def codex_home(environ: dict) -> Optional[Path]:
    explicit = environ.get("CODEX_HOME")
    if explicit:
        return Path(explicit)
    home = environ.get("HOME") or environ.get("USERPROFILE")
    return Path(home) / ".codex" if home else None


def check_login(environ: dict) -> None:
    home = codex_home(environ)
    if home is None:
        raise Refusal(f"не найден домашний каталог: задайте CODEX_HOME. {FALLBACK}")
    if not (home / "auth.json").is_file():
        raise Refusal(
            f"вход в Codex не выполнен: нет {home / 'auth.json'}. "
            f"Выполните `codex login`. {FALLBACK}"
        )


def git(root: Path, *args: str) -> str:
    done = subprocess.run(
        ["git", *args],
        cwd=str(root),
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if done.returncode != 0:
        detail = done.stderr.strip() or done.stdout.strip() or f"код {done.returncode}"
        raise Refusal(f"git {' '.join(args)}: {detail}")
    return done.stdout


def review_args(base: Optional[str], commit: Optional[str], title: Optional[str]) -> List[str]:
    args = ["review"]
    if commit:
        args += ["--commit", commit]
    else:
        args += ["--base", base or DEFAULT_BASE]
    if title:
        args += ["--title", title]
    return args


def out_path(root: Path, ref: str, now: float) -> Path:
    tag = "".join(char if char.isalnum() or char in "-._" else "-" for char in ref)
    stamp = time.strftime("%Y%m%d-%H%M%S", time.localtime(now))
    return root / OUT_DIR / f"{stamp}-{tag[:40]}.md"


def group_options() -> dict:
    """Запуск своей группой процессов, чтобы потом убить её целиком."""
    if sys.platform == "win32":
        return {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP}
    return {"start_new_session": True}


def kill_tree(process: "subprocess.Popen") -> None:
    """Убить Codex вместе с потомками: оболочка PATH, его агент и shell."""
    if sys.platform == "win32":
        subprocess.run(
            ["taskkill", "/T", "/F", "/PID", str(process.pid)],
            capture_output=True,
            check=False,
        )
    else:
        try:
            os.killpg(os.getpgid(process.pid), signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass
    process.kill()
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        pass


def run(
    command: Sequence[str],
    cwd: Path,
    env: dict,
    destination: Path,
    echo: Callable[[str], None],
    timeout: int,
) -> int:
    """Вывод Codex идёт в файл и на stderr; stdout остаётся под путь к файлу.

    Вывод читает отдельный поток: зависший Codex может не напечатать ни
    строки, и предел времени должен сработать всё равно. По пределу времени
    убивается всё дерево процессов: `codex` может оказаться оболочкой
    (`codex.cmd` в PATH) или сам держать потомков, и тогда смерть одного
    процесса оставила бы ревью работать, а чтение — висеть на трубе.
    """
    destination.parent.mkdir(parents=True, exist_ok=True)
    process = subprocess.Popen(
        list(command),
        cwd=str(cwd),
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
        **group_options(),
    )
    assert process.stdout is not None
    with destination.open("w", encoding="utf-8", newline="\n") as sink:

        def pump() -> None:
            for line in process.stdout:  # type: ignore[union-attr]
                sink.write(line)
                sink.flush()
                echo(line.rstrip("\n"))

        reader = threading.Thread(target=pump, daemon=True)
        reader.start()
        try:
            code = process.wait(timeout=timeout or None)
        except subprocess.TimeoutExpired:
            kill_tree(process)
            code = 124
        reader.join(timeout=30)
        # Закрывать трубу под ещё читающим потоком нельзя: закрытие повиснет.
        if not reader.is_alive():
            process.stdout.close()
        if code == 124:
            sink.write(f"\ncodex-review: Codex убит по пределу времени {timeout} с\n")
    return code


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--base", help=f"ветка базы, по умолчанию {DEFAULT_BASE}")
    source.add_argument("--commit", help="ревью одного коммита вместо диапазона от базы")
    parser.add_argument("--title", help="заголовок, который Codex покажет в итоге")
    parser.add_argument("--out", help="файл вывода вместо .session-temp/codex-review/")
    parser.add_argument(
        "--timeout",
        type=int,
        default=DEFAULT_TIMEOUT,
        help=f"секунд без завершения, после которых Codex убивается (0 — без предела, по умолчанию {DEFAULT_TIMEOUT})",
    )
    args = parser.parse_args(argv)

    environ = dict(os.environ)
    ref = args.commit or args.base or DEFAULT_BASE
    try:
        codex = find_codex(environ, sys.platform, shutil.which)
        check_login(environ)
        # Корень рабочей копии, а не REPO_ROOT: скрипт зовут и из подкаталога.
        root = Path(git(REPO_ROOT, "rev-parse", "--show-toplevel").strip())
        try:
            git(root, "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}")
        except Refusal:
            raise Refusal(f"нет коммита или ветки {ref}: сделайте `git fetch` или назовите другую базу")
    except Refusal as error:
        print(f"codex-review: {error}", file=sys.stderr)
        return 2

    command = [str(codex), *review_args(args.base, args.commit, args.title)]
    destination = Path(args.out) if args.out else out_path(root, ref, time.time())
    if not destination.is_absolute():
        destination = Path.cwd() / destination

    print(f"codex-review: {' '.join(command[1:])} в {root}", file=sys.stderr)
    code = run(
        command,
        root,
        {**environ, "PYTHONUTF8": "1"},
        destination,
        lambda line: print(line, file=sys.stderr),
        args.timeout,
    )
    print(destination)
    if code == 124:
        print(f"codex-review: Codex не завершился за {args.timeout} с и убит", file=sys.stderr)
        return 1
    if code != 0:
        print(f"codex-review: Codex завершился с кодом {code}; {FALLBACK}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
