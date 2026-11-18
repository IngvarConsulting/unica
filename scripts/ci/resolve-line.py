#!/usr/bin/env python3
"""Линия прогона: ветка для push в ветку, база для очереди, релизная линия для тега.

У тега нет ветки, а линия нужна: по ней сайт кладёт отчёт и память ночного
прогона. Линия тега — та `release-vX.Y`, что содержит его коммит. Теги кнопок
выпуска стоят на служебном коммите версии, который не лежит ни в одной ветке,
и линию называет его родитель: кандидат `vX.Y.Z-rc.N` стоит на срезе `main`
(линия `main`), исправительная `vX.Y.Z` — на голове своей `release-vX.Y`.
Финальная версия создаёт свою линию на собственном коммите. Тег, чей коммит
лежит в ветке, но не в релизной линии, и тег вне веток, родитель которого не
в линии, допустимой его именем, — отказ.

Очередь слияния идёт на временной ветке `gh-readonly-queue/<база>/pr-N-<sha>`:
её линия — база. Прогон очереди проверяет то дерево, что ляжет в базу, и его
отчёт сайт кладёт в линию базы, а не в ветку, которой через минуту не станет.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys

LINE = re.compile(r"\Arelease-v(\d+)\.(\d+)\Z")
ALPHA_TAG = re.compile(r"\Av\d+\.\d+\.\d+-alpha\.\d+\Z")
QUEUE_BRANCH = re.compile(r"\Agh-readonly-queue/(?P<base>[^/]+)/pr-\d+-[0-9a-f]+\Z")


# Тег кнопки на служебном коммите вне веток: кандидат стоит на срезе `main`,
# исправительная `vX.Y.Z` — на голове `release-vX.Y`. Имя тега задаёт, какая
# линия под ним допустима; другой предок — отказ.
RC_TAG = re.compile(r"\Av\d+\.\d+\.\d+-rc\.\d+\Z")
PATCH_TAG = re.compile(r"\Av(?P<major>\d+)\.(?P<minor>\d+)\.\d+\Z")


def git_first_parent(sha: str) -> str | None:
    completed = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", f"{sha}^"],
        capture_output=True, text=True, check=False,
    )
    return completed.stdout.strip() or None


def git_branches_containing(sha: str) -> list[str]:
    completed = subprocess.run(
        ["git", "branch", "-r", "--contains", sha, "--format=%(refname:short)"],
        capture_output=True, text=True, check=True,
    )
    return [name.removeprefix("origin/") for name in completed.stdout.split()]


def release_line(names: list[str]) -> str | None:
    lines = [name for name in names if LINE.match(name)]
    # Коммит может лежать в нескольких линиях, если старшая ответвилась позже:
    # хозяйка — младшая по версии, остальные его лишь унаследовали.
    return min(lines, key=lambda name: tuple(int(part) for part in LINE.match(name).groups())) if lines else None


def resolve(ref_type: str, ref_name: str, sha: str, branches_containing=git_branches_containing,
            first_parent=git_first_parent) -> str:
    if ref_type == "branch":
        queued = QUEUE_BRANCH.match(ref_name)
        return queued.group("base") if queued else ref_name
    if ref_type != "tag":
        raise SystemExit(f"неизвестный тип ссылки {ref_type!r}: ожидались branch или tag")
    # Alpha — срез main под служебным коммитом вне веток: его линия main.
    if ALPHA_TAG.match(ref_name):
        return "main"
    refusal = (f"тег {ref_name} стоит на {sha[:7]}, которого нет ни в одной релизной линии release-vX.Y: "
               "теги ставятся только на релизных линиях")
    branches = branches_containing(sha)
    if branches:
        line = release_line(branches)
        if line is None:
            raise SystemExit(refusal)
        return line
    # Служебный коммит кнопки: линия — та, что держит его родителя и которую
    # допускает имя тега.
    parent = first_parent(sha)
    holders = branches_containing(parent) if parent else []
    if RC_TAG.match(ref_name) and "main" in holders:
        return "main"
    patch = PATCH_TAG.match(ref_name)
    if patch and f"release-v{patch['major']}.{patch['minor']}" in holders:
        return f"release-v{patch['major']}.{patch['minor']}"
    raise SystemExit(refusal)


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
