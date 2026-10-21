#!/usr/bin/env python3
"""Дождаться джоб других ОС того же прогона и напечатать их исходы.

Запись очереди занимает два раннера: Linux-джобу `Unica CI` и джобу macOS.
`needs` между ними сделал бы прогон последовательным, поэтому `Unica CI`
идёт параллельно и в конце читает исход джобы `Rust (<runner>)` из jobs API
этого же прогона. Скрипт ничего не решает: исходы сверяет `ci_rust_lanes`
(`CI_JOB_RESULTS`) и оценка гейта. Не дождались — исход `timed_out`, джобы
нет среди завершённых — `missing`. Отменённая джоба без раннера — это
`not_acquired`: GitHub так и не выдал раннер этого типа («The job was not
acquired by Runner … even after multiple attempts»), а не потеря её части.
Ни один из этих исходов успехом не является.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from typing import Callable

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ci_rust_lanes import GATE_JOB, LINUX, os_job  # noqa: E402


def list_jobs(repository: str, run_id: str) -> list[dict]:
    pages = subprocess.check_output(
        ["gh", "api", "--paginate", f"repos/{repository}/actions/runs/{run_id}/jobs?filter=latest&per_page=100",
         "--jq", ".jobs[] | {name, status, conclusion, runner_name}"], text=True,
    )
    return [json.loads(line) for line in pages.splitlines() if line.strip()]


NOT_ACQUIRED = "not_acquired"


def conclusion(job: dict) -> str:
    outcome = job.get("conclusion") or "unknown"
    # A hosted runner never picked the job up: no runner name is recorded.
    if outcome == "cancelled" and not job.get("runner_name"):
        return NOT_ACQUIRED
    return outcome


def outcomes(jobs: list[dict], expected: list[str]) -> dict[str, str] | None:
    """Исходы ожидаемых джоб или None, пока какая-то из них может появиться или идёт."""
    found = {job["name"]: job for job in jobs}
    result = {}
    for name in expected:
        job = found.get(name)
        if job is None:
            # Матрица ещё не развёрнута или джобы нет вовсе: «нет» становится
            # окончательным, только когда больше ничего, кроме гейта, не идёт.
            if any(item["status"] != "completed" for item in jobs if item["name"] != GATE_JOB):
                return None
            result[name] = "missing"
        elif job["status"] != "completed":
            return None
        else:
            result[name] = conclusion(job)
    return result


def wait(expected: list[str], read: Callable[[], list[dict]], *, timeout: float, interval: float,
         clock: Callable[[], float] = time.monotonic, sleep: Callable[[float], None] = time.sleep) -> dict[str, str]:
    deadline = clock() + timeout
    jobs: list[dict] = []
    while True:
        try:
            jobs = read()
            current = outcomes(jobs, expected)
        except (subprocess.CalledProcessError, json.JSONDecodeError, KeyError) as error:
            print(f"jobs API: {error}", file=sys.stderr, flush=True)
            current = None
        if current is not None:
            return current
        if clock() >= deadline:
            # Завершённая джоба сохраняет свой исход; срок истёк у остальных.
            found = {job["name"]: job for job in jobs}
            return {name: conclusion(found[name]) if name in found and found[name]["status"] == "completed"
                    else "timed_out" for name in expected}
        sleep(interval)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--runners", required=True, help="JSON runners of this gate profile")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=3600)
    parser.add_argument("--interval", type=float, default=20)
    args = parser.parse_args()
    expected = [os_job(host) for host in json.loads(args.runners) if host != LINUX]
    repository, run_id = os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_RUN_ID"]
    result = wait(expected, lambda: list_jobs(repository, run_id), timeout=args.timeout, interval=args.interval) if expected else {}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, separators=(",", ":")) + "\n", encoding="utf-8")
    print(json.dumps(result, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
