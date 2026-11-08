#!/usr/bin/env python3
"""Исследовательские тесты: только вручную, вне ворот конвейера.

    python3.12 scripts/dev/run-research.py --profile research --issue <номер>
        Воспроизведения одной нерешённой issue: цель `repro` крейта
        `crates/unica-research` (тесты `issue_<номер>…`) и модули
        `tests/research/repro/test_issue_<номер>_*.py`.

    python3.12 scripts/dev/run-research.py --profile platform
        Замеры платформы 1С, скорости и окружения: цель `platform` и
        `tests/research/platform/`. Предусловия называет сам тест.

Регресс запускает `scripts/ci/run-tests.py` профилями ворот; исследовательского
места он не знает. Класс теста задаёт его место, отбора по спискам нет.

Крейт `unica-research` вне рабочего пространства, поэтому у него свой
`Cargo.lock`. Перед сборкой он берётся копией корневого: версии зависимостей
те же, что у продукта.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATE = Path("crates/unica-research")
PYTHON_ROOT = Path("tests/research")
PROFILES = {"research": "repro", "platform": "platform"}


def rust_command(profile: str, issue: int | None) -> list[str]:
    target = PROFILES[profile]
    command = ["cargo", "nextest", "run", "--manifest-path", str(CRATE / "Cargo.toml"),
               "--test", target, "--no-tests=warn"]
    if issue is not None:
        command += ["-E", f"test(/^issue_{issue}(_|::|$)/)"]
    return command


def python_command(profile: str, issue: int | None) -> list[str]:
    target = PROFILES[profile]
    pattern = f"test_issue_{issue}_*.py" if issue is not None else "test_*.py"
    return [sys.executable, "-m", "unittest", "discover", "-s", str(PYTHON_ROOT / target),
            "-t", ".", "-p", pattern]


def commands(profile: str, issue: int | None, ecosystem: str) -> list[list[str]]:
    found = []
    if ecosystem in ("all", "rust"):
        found.append(rust_command(profile, issue))
    if ecosystem in ("all", "python"):
        found.append(python_command(profile, issue))
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--profile", required=True, choices=sorted(PROFILES))
    parser.add_argument("--issue", type=int, help="номер issue; обязателен для research")
    parser.add_argument("--ecosystem", default="all", choices=("all", "rust", "python"))
    parser.add_argument("--dry-run", action="store_true", help="напечатать команды и выйти")
    args = parser.parse_args(argv)
    if args.profile == "research" and args.issue is None:
        parser.error("--profile research требует --issue")
    if args.profile == "platform" and args.issue is not None:
        parser.error("--issue относится только к research")

    planned = commands(args.profile, args.issue, args.ecosystem)
    if args.dry_run:
        for command in planned:
            print(" ".join(command))
        return 0
    if args.ecosystem in ("all", "rust"):
        shutil.copyfile(ROOT / "Cargo.lock", ROOT / CRATE / "Cargo.lock")
    status = 0
    for command in planned:
        completed = subprocess.run(command, cwd=ROOT, env={**os.environ, "PYTHONUTF8": "1"}, check=False)
        status = status or completed.returncode
    return status


if __name__ == "__main__":
    sys.exit(main())
