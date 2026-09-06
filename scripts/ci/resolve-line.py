#!/usr/bin/env python3
"""Линия прогона: ветка для push в ветку, релизная линия для тега.

У тега нет ветки, а линия нужна: по ней сайт кладёт отчёт и память ночного
прогона. Линия тега — та `release-vX.Y`, что содержит его коммит; тег вне
релизной линии — отказ. Это практика «теги только на релизных линиях», под
которую лёг замысел площадки, и страж держит её на входе, а не по памяти.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys

LINE = re.compile(r"\Arelease-v(\d+)\.(\d+)\Z")


def git_branches_containing(sha: str) -> list[str]:
    completed = subprocess.run(
        ["git", "branch", "-r", "--contains", sha, "--format=%(refname:short)"],
        capture_output=True, text=True, check=True,
    )
    return [name.removeprefix("origin/") for name in completed.stdout.split()]


def resolve(ref_type: str, ref_name: str, sha: str, branches_containing=git_branches_containing) -> str:
    if ref_type == "branch":
        return ref_name
    if ref_type != "tag":
        raise SystemExit(f"неизвестный тип ссылки {ref_type!r}: ожидались branch или tag")
    lines = [name for name in branches_containing(sha) if LINE.match(name)]
    if not lines:
        raise SystemExit(
            f"тег {ref_name} стоит на {sha[:7]}, которого нет ни в одной релизной линии release-vX.Y: "
            "теги ставятся только на релизных линиях"
        )
    # Коммит может лежать в нескольких линиях, если старшая ответвилась позже:
    # хозяйка — младшая по версии, остальные его лишь унаследовали.
    return min(lines, key=lambda name: tuple(int(part) for part in LINE.match(name).groups()))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--ref-type", required=True, help="branch или tag (github.ref_type)")
    parser.add_argument("--ref-name", required=True, help="имя ветки или тега (github.ref_name)")
    parser.add_argument("--sha", required=True, help="коммит прогона")
    args = parser.parse_args(argv)
    print(resolve(args.ref_type, args.ref_name, args.sha))
    return 0


if __name__ == "__main__":
    sys.exit(main())
