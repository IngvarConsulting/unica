#!/usr/bin/env python3
"""Дождаться джоб других ОС того же прогона и напечатать их исходы.

Запись очереди занимает два раннера: Linux-джобу `Unica CI` и джобу macOS.
`needs` между ними сделал бы прогон последовательным, поэтому `Unica CI`
идёт параллельно и в конце читает исход джобы `Rust (<runner>)` из jobs API
этого же прогона. Скрипт ничего не решает: исходы сверяет `ci_rust_lanes`
(`CI_JOB_RESULTS`) и оценка гейта. Не дождались — исход `timed_out`, в том
числе у завершённой джобы, чей итог так и не записан; шаг завершённой джобы
без итога по сроку получает итог джобы; джобы нет среди завершённых —
`missing`. Отменённая джоба без раннера — это
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
         "--jq", ".jobs[] | {name, status, conclusion, runner_name, steps: [.steps[]? | {name, conclusion}]}"], text=True,
    )
    return [json.loads(line) for line in pages.splitlines() if line.strip()]


NOT_ACQUIRED = "not_acquired"


def conclusion(job: dict) -> str:
    outcome = job.get("conclusion") or "unknown"
    # A hosted runner never picked the job up: no runner name is recorded.
    if outcome == "cancelled" and not job.get("runner_name"):
        return NOT_ACQUIRED
    return outcome


STEP = "::"


def outcome(name: str, found: dict[str, dict], jobs: list[dict], *, final: bool = False) -> str | None:
    """Исход одной ожидаемой джобы или шага; None, пока он может измениться.

    Jobs API отдаёт `status=completed` раньше, чем заполняет итоги: пустой
    итог у завершённой джобы или её шага значит «ещё не записан», а не
    провал (#2008). Шаг завершённой джобы может долго оставаться без итога
    или вовсе не попасть в список (#2026): до срока это ожидание, а по сроку
    (`final`) итог шага — итог джобы, так что отказ джобы не становится
    успехом шага.
    """
    job_name, _, step_name = name.partition(STEP)
    job = found.get(job_name)
    if job is None:
        # Матрица ещё не развёрнута или джобы нет вовсе: «нет» становится
        # окончательным, только когда больше ничего, кроме гейта, не идёт.
        if any(item["status"] != "completed" for item in jobs if item["name"] != GATE_JOB):
            return None
        return "missing"
    if job["status"] != "completed" or not job.get("conclusion"):
        return None
    if step_name and conclusion(job) != NOT_ACQUIRED and job.get("steps"):
        step = next((item for item in job["steps"] if item.get("name") == step_name), None)
        written = step.get("conclusion") if step else None
        return written or (conclusion(job) if final else None)
    # Jobs API бывает отдаёт завершённую джобу вовсе без шагов (PR #2012):
    # тогда итог шага — итог самой джобы. Успешная джоба значит успешные
    # шаги, а отказ джобы не превращается в успех ни одного из них.
    return conclusion(job)


def outcomes(jobs: list[dict], expected: list[str]) -> dict[str, str] | None:
    """Исходы ожидаемых джоб или None, пока какая-то из них может появиться или идёт.

    `Джоба::Шаг` — исход одного шага джобы: у джобы pull request с Clippy
    и Python отказ одного не должен выдавать себя за отказ другого, пока
    итог каждого записан; незаписанный по сроку берёт итог джобы.
    """
    found = {job["name"]: job for job in jobs}
    result = {name: outcome(name, found, jobs) for name in expected}
    return None if None in result.values() else result


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
            # Записанный исход найденной джобы сохраняется; срок истёк у остальных.
            found = {job["name"]: job for job in jobs}
            return {name: (outcome(name, found, jobs, final=True) if name.partition(STEP)[0] in found else None)
                    or "timed_out" for name in expected}
        sleep(interval)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--runners", required=True, help="JSON runners of this gate profile")
    parser.add_argument("--job", action="append", default=[], help="another job of this run to await by its name; JOB::STEP for one step")
    parser.add_argument("--require-success", action="store_true", help="exit 1 unless every awaited job succeeded")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=3600)
    parser.add_argument("--interval", type=float, default=20)
    args = parser.parse_args()
    expected = [os_job(host) for host in json.loads(args.runners) if host != LINUX] + args.job
    repository, run_id = os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_RUN_ID"]
    result = wait(expected, lambda: list_jobs(repository, run_id), timeout=args.timeout, interval=args.interval) if expected else {}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, separators=(",", ":")) + "\n", encoding="utf-8")
    print(json.dumps(result, separators=(",", ":")))
    return 1 if args.require_success and any(outcome != "success" for outcome in result.values()) else 0


if __name__ == "__main__":
    sys.exit(main())
