#!/usr/bin/env python3
"""Перечислить линии, чья вершина сдвинулась с прошлого прогона `large`, и запустить его.

Расписание у GitHub работает только на ветке по умолчанию, поэтому ночной
workflow один: он перечисляет открытые линии тем же правилом, что и сайт, и
для каждой сдвинувшейся запускает `unica-large.yml` на самой линии — там
`github.sha` и `github.ref_name` и есть вершина и линия, и checkout стандартный.
Память о прошлом прогоне лежит на сайте: `data/<линия>/profiles/large.json`
— коммит, время и ссылка. Прогон по тегу пишет ту же память, поэтому после
тега на вершине ночью — пропуск. Линия без `unica-large.yml` площадки не
несёт и в ночь не идёт.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

LARGE_WORKFLOW = "unica-large.yml"


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


def has_platform(repo: str, line: str, gh) -> bool:
    """Несёт ли линия ночной workflow; без него запускать нечего."""
    try:
        gh(repo, f"contents/.github/workflows/{LARGE_WORKFLOW}?ref={line}")
    except subprocess.CalledProcessError:
        return False
    return True


def dispatch_large(line: str) -> None:
    # `gh` печатает адрес прогона в stdout, а stdout этого скрипта — строки
    # для GITHUB_OUTPUT: чужая строка там — отказ раннера. Адрес — в stderr.
    completed = subprocess.run(
        ["gh", "workflow", "run", LARGE_WORKFLOW, "--ref", line], check=True, capture_output=True, text=True
    )
    print(f"{line}: {completed.stdout.strip() or 'запущен'}", file=sys.stderr)


def list_large_runs(line: str) -> list[dict]:
    completed = subprocess.run(
        ["gh", "run", "list", "--workflow", LARGE_WORKFLOW, "--branch", line, "--event", "workflow_dispatch",
         "--limit", "5", "--json", "databaseId,createdAt,status,conclusion"],
        check=True, capture_output=True, text=True,
    )
    return json.loads(completed.stdout)


def find_started_run(line: str, since: str, list_runs=list_large_runs, attempts: int = 30, pause: float = 10.0) -> int:
    """Прогон, созданный после запуска: `gh workflow run` его номера не отдаёт."""
    for _ in range(attempts):
        runs = [run for run in list_runs(line) if run.get("createdAt", "") >= since]
        if runs:
            return int(max(runs, key=lambda run: run["createdAt"])["databaseId"])
        time.sleep(pause)
    raise SystemExit(f"{line}: запущенный прогон {LARGE_WORKFLOW} не появился в списке за {attempts * pause:.0f} с")


def watch_run(run_id: int) -> str:
    """Дождаться прогона и вернуть его заключение; красный — тоже результат."""
    subprocess.run(["gh", "run", "watch", str(run_id), "--interval", "30"], check=False, capture_output=True, text=True)
    completed = subprocess.run(
        ["gh", "run", "view", str(run_id), "--json", "conclusion", "--jq", ".conclusion"],
        check=True, capture_output=True, text=True,
    )
    return completed.stdout.strip()


def download_run(run_id: int, dest: Path) -> None:
    dest.mkdir(parents=True, exist_ok=True)
    subprocess.run(["gh", "run", "download", str(run_id), "--dir", str(dest)], check=True, capture_output=True, text=True)


def follow(decisions: list[dict], dest: Path, *, find=find_started_run, watch=watch_run, download=download_run) -> list[dict]:
    """Дождаться запущенных прогонов и забрать их артефакты в `dest/<линия>/`.

    Событие завершения прогона, созданного `GITHUB_TOKEN`, другие workflow не
    будит, поэтому ночь несёт их артефакты сама: внутри те же `results-*` и
    `plan-*` с подписями, и сайт находит их на любой глубине.
    """
    outcomes = []
    for decision in decisions:
        if not decision.get("since"):
            continue
        run_id = find(decision["line"], decision["since"])
        conclusion = watch(run_id)
        download(run_id, dest / decision["line"])
        decision["reason"] += f", прогон {run_id}: {conclusion}"
        outcomes.append({"line": decision["line"], "run_id": run_id, "conclusion": conclusion})
    return outcomes


def enumerate_lines(repo: str, site: str, now: datetime, *, gh, fetch, open_lines, platform=None) -> list[dict]:
    """Каждая открытая линия с решением: гнать или пропустить, и почему."""
    platform = platform or (lambda line: has_platform(repo, line, gh))
    decisions = []
    for line in ["main", *open_lines(repo, now)]:
        sha = head_sha(repo, line, gh)
        if not platform(line):
            decisions.append({"line": line, "sha": sha, "run": False, "reason": f"линия без площадки: нет {LARGE_WORKFLOW}"})
            continue
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


def dispatch(decisions: list[dict], run_workflow=dispatch_large) -> list[str]:
    """Запустить `large` на каждой линии со сдвигом; вернуть запущенные."""
    started = []
    for decision in decisions:
        if not decision["run"]:
            continue
        # Метка «до запуска» с запасом: по ней потом находится созданный прогон.
        decision["since"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() - 60))
        run_workflow(decision["line"])
        decision["reason"] += " — запущен"
        started.append(decision["line"])
    return started


def summary(decisions: list[dict]) -> str:
    rows = "\n".join(f"| `{d['line']}` | {'гоним' if d['run'] else 'пропуск'} | {d['reason']} |" for d in decisions)
    return "| Линия | Решение | Почему |\n| --- | --- | --- |\n" + rows + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", required=True)
    parser.add_argument("--site", required=True, help="адрес сайта с памятью прогонов")
    parser.add_argument("--dispatch", action="store_true", help="запустить unica-large.yml на сдвинувшихся линиях")
    parser.add_argument("--follow", type=Path, default=None, help="дождаться запущенных прогонов и забрать их артефакты сюда")
    parser.add_argument("--summary", type=Path, default=None, help="куда дописать таблицу решений")
    args = parser.parse_args(argv)

    status = load_site_status()
    decisions = enumerate_lines(
        args.repo, args.site, datetime.now(timezone.utc), gh=status.gh, fetch=fetch_json, open_lines=status.open_lines
    )
    started = dispatch(decisions) if args.dispatch else [d["line"] for d in decisions if d["run"]]
    if args.dispatch and args.follow is not None:
        follow(decisions, args.follow)
    table = summary(decisions)
    if args.summary is not None:
        with args.summary.open("a", encoding="utf-8") as handle:
            handle.write("## Ночной прогон: линии\n\n" + table)
    print(table, file=sys.stderr)
    # Строки для GITHUB_OUTPUT: линии со сдвигом и их число.
    print(f"lines={' '.join(started)}")
    print(f"count={len(started)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
