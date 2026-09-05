#!/usr/bin/env python3
"""Перечислить линии, чья вершина сдвинулась с прошлого прогона `large`.

Расписание у GitHub работает только на ветке по умолчанию, поэтому ночной
workflow один и сам перечисляет открытые линии тем же правилом, что и сайт.
Память о прошлом прогоне лежит на сайте: `data/<линия>/profiles/large.json` —
коммит, время и ссылка. Линия идёт в матрицу, если памяти нет или её коммит
не равен вершине линии. Прогон по тегу пишет ту же память, поэтому после тега
на вершине ночью — пропуск.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

RUNNERS = ("ubuntu-latest", "macos-14")


def load_site_status():
    spec = importlib.util.spec_from_file_location("site_status", Path(__file__).with_name("site-status.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def fetch_json(url: str):
    """JSON с опубликованного сайта; `None`, когда файла там нет."""
    try:
        with urllib.request.urlopen(url, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise


def head_sha(repo: str, line: str, gh) -> str:
    data = gh(repo, f"branches/{line}")
    if isinstance(data, list):
        data = data[0]
    return data["commit"]["sha"]


def enumerate_lines(repo: str, site: str, now: datetime, *, gh, fetch, open_lines) -> list[dict]:
    """Каждая открытая линия с решением: гнать или пропустить, и почему."""
    decisions = []
    for line in ["main", *open_lines(repo, now)]:
        sha = head_sha(repo, line, gh)
        memory = fetch(f"{site}/data/{line}/profiles/large.json") or {}
        last = memory.get("sha", "")
        if not last:
            reason, run = "памяти о прошлом прогоне нет", True
        elif last != sha:
            reason, run = f"вершина сдвинулась: {last[:7]} → {sha[:7]}", True
        else:
            reason, run = f"вершина на месте: {sha[:7]} уже проверен ({memory.get('at', '?')})", False
        decisions.append({"line": line, "sha": sha, "run": run, "reason": reason})
    return decisions


def matrix(decisions: list[dict], runners: tuple[str, ...] = RUNNERS) -> dict:
    """Полные сочетания линия × раннер: `include` без общих ключей матрица не сложит."""
    return {
        "include": [
            {"line": d["line"], "sha": d["sha"], "runner": runner}
            for d in decisions
            if d["run"]
            for runner in runners
        ]
    }


def summary(decisions: list[dict]) -> str:
    rows = "\n".join(f"| `{d['line']}` | {'гоним' if d['run'] else 'пропуск'} | {d['reason']} |" for d in decisions)
    return "| Линия | Решение | Почему |\n| --- | --- | --- |\n" + rows + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", required=True)
    parser.add_argument("--site", required=True, help="адрес сайта с памятью прогонов")
    parser.add_argument("--out", type=Path, required=True, help="куда записать матрицу")
    parser.add_argument("--summary", type=Path, default=None, help="куда дописать таблицу решений")
    args = parser.parse_args(argv)

    status = load_site_status()
    decisions = enumerate_lines(
        args.repo, args.site, datetime.now(timezone.utc), gh=status.gh, fetch=fetch_json, open_lines=status.open_lines
    )
    chosen = matrix(decisions)
    args.out.write_text(json.dumps(chosen, ensure_ascii=False) + "\n", encoding="utf-8")
    table = summary(decisions)
    if args.summary is not None:
        with args.summary.open("a", encoding="utf-8") as handle:
            handle.write("## Ночной прогон: линии\n\n" + table)
    print(table, file=sys.stderr)
    # Строки для GITHUB_OUTPUT: матрица одной строкой и число сочетаний.
    print(f"matrix={json.dumps(chosen, ensure_ascii=False)}")
    print(f"count={len(chosen['include'])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
