#!/usr/bin/env python3
"""Решить, гнать ли ночной полный прогон вершины main.

Память о прошлом прогоне лежит на сайте: `data/main/profiles/large.json` — коммит,
время и ссылка. Вершина, уже проверенная таким прогоном, ночью не гоняется;
кнопка с `--force` прогоняет её снова. Сайт недоступен или памяти нет — гоним:
лишний прогон дешевле пропущенной ночи.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path


def fetch_json(url: str):
    """JSON с опубликованного сайта; `None`, когда его не прочитать."""
    try:
        with urllib.request.urlopen(url, timeout=30) as response:
            return json.load(response)
    except (urllib.error.URLError, TimeoutError, ValueError):
        return None


def decide(sha: str, memory: dict | None, *, force: bool = False) -> tuple[bool, str]:
    last = (memory or {}).get("sha", "")
    if force:
        return True, f"кнопка с force: {sha[:7]}"
    if not last:
        return True, "памяти о прошлом прогоне нет"
    if last != sha:
        return True, f"вершина сдвинулась: {last[:7]} → {sha[:7]}"
    return False, f"вершина на месте: {sha[:7]} уже проверен ({(memory or {}).get('at', '?')})"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--site", required=True, help="адрес сайта с памятью прогонов")
    parser.add_argument("--sha", required=True, help="вершина main этого прогона")
    parser.add_argument("--force", action="store_true", help="гнать, даже если вершина уже проверена")
    parser.add_argument("--summary", type=Path, default=None, help="куда дописать решение")
    args = parser.parse_args(argv)
    run, reason = decide(args.sha, fetch_json(f"{args.site}/data/main/profiles/large.json"), force=args.force)
    line = f"{'гоним' if run else 'пропуск'}: {reason}"
    print(line, file=sys.stderr)
    if args.summary is not None:
        with args.summary.open("a", encoding="utf-8") as handle:
            handle.write(f"## Ночной прогон\n\n{line}\n")
    # Строка для GITHUB_OUTPUT.
    print(f"run={str(run).lower()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
