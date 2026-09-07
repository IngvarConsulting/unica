#!/usr/bin/env python3
"""Критерии быстроты и надёжности конвейера, посчитанные по прогонам.

Числа берутся из прогонов GitHub Actions, вливаний и трендов отчёта на
сайте за скользящее окно, и ни одно не пишется руками. Пороги — из плана
`docs/design/2026-09-07-pipeline-parallel-sessions-plan-design.md`; страница
«Конвейер» показывает значение рядом с целью, чтобы расхождение было видно
раньше, чем его заметят по ощущениям.
"""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import urllib.request
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from pathlib import Path

BUILD_WORKFLOW = "unica-plugin-release.yml"
PAGES_WORKFLOW = "unica-pages.yml"
WINDOW_DAYS = 14
# Минуты раннеров считаются по джобам, а это вызов на прогон: берётся хвост.
RUNNER_MINUTES_RUNS = 60


def moment(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def minutes(start: str, end: str) -> float:
    return (moment(end) - moment(start)).total_seconds() / 60


def gh_json(args: list[str]) -> list | dict:
    completed = subprocess.run(["gh", *args], capture_output=True, text=True, check=True)
    return json.loads(completed.stdout or "[]")


def fetch_json(url: str):
    """Файл сайта или `None`: метрика без источника показывает прочерк, а не ноль."""
    try:
        with urllib.request.urlopen(url, timeout=30) as response:
            return json.loads(response.read().decode("utf-8"))
    except Exception:
        return None


def percentile(values: list[float], share: float) -> float:
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, round(share * (len(ordered) - 1))))
    return ordered[index]


def fmt_minutes(value: float | None) -> str:
    return "—" if value is None else f"{value:.1f} мин".replace(".", ",")


def fmt_share(part: int, whole: int) -> str:
    return "—" if whole == 0 else f"{part} из {whole} ({100 * part / whole:.0f} %)"


def load_runs(gh, repo: str, since: datetime) -> dict[str, list[dict]]:
    """Прогоны сборки за окно, по событиям; список сайта — отдельно."""
    fields = "databaseId,event,headBranch,headSha,status,conclusion,createdAt,startedAt,updatedAt"
    runs = gh(["run", "list", "--repo", repo, "--workflow", BUILD_WORKFLOW, "--limit", "600", "--json", fields])
    pages = gh(["run", "list", "--repo", repo, "--workflow", PAGES_WORKFLOW, "--limit", "300", "--json", fields])
    recent = [run for run in runs if run["status"] == "completed" and moment(run["createdAt"]) >= since]
    by_event: dict[str, list[dict]] = defaultdict(list)
    for run in recent:
        by_event[run["event"]].append(run)
    by_event["pages"] = [run for run in pages if run["status"] == "completed" and moment(run["createdAt"]) >= since]
    return by_event


def load_merged(gh, repo: str, since: datetime) -> list[dict]:
    merged = gh(["pr", "list", "--repo", repo, "--state", "merged", "--limit", "300", "--json", "number,createdAt,mergedAt,headRefName,mergeCommit"])
    return [pr for pr in merged if pr.get("mergedAt") and moment(pr["mergedAt"]) >= since]


def runner_minutes(gh, repo: str, runs: list[dict]) -> float | None:
    """Сумма длительностей джоб по последним прогонам: минуты раннеров, не стена."""
    total = 0.0
    counted = 0
    for run in sorted(runs, key=lambda r: r["createdAt"], reverse=True)[:RUNNER_MINUTES_RUNS]:
        detail = gh(["run", "view", "--repo", repo, str(run["databaseId"]), "--json", "jobs"])
        for job in detail.get("jobs", []):
            if job.get("conclusion") in (None, "skipped") or not job.get("completedAt") or not job.get("startedAt"):
                continue
            total += minutes(job["startedAt"], job["completedAt"])
        counted += 1
    return None if counted == 0 else total / counted


def compute(by_event: dict[str, list[dict]], merged: list[dict], retry_trend, summary, gh=None, repo: str = "") -> list[dict]:
    """Строки страницы: критерий, значение, цель, источник."""
    prs = [r for r in by_event.get("pull_request", []) if r["conclusion"] in ("success", "failure")]
    mains = [r for r in by_event.get("push", []) if r["headBranch"] == "main" and r["conclusion"] in ("success", "failure")]
    queue = by_event.get("merge_group", [])
    pages = by_event.get("pages", [])
    rows: list[dict] = []

    signals = [minutes(r["startedAt"], r["updatedAt"]) for r in prs]
    rows.append({
        "metric": "Сигнал PR, медиана и 90-й процентиль",
        "value": "—" if not signals else f"{fmt_minutes(statistics.median(signals))} / {fmt_minutes(percentile(signals, 0.9))}",
        "target": "4 / 6 мин",
        "source": f"{len(signals)} завершённых прогонов pull request",
    })

    push_to_merge: list[float] = []
    for pr in merged:
        heads = [r for r in by_event.get("pull_request", []) if r["headBranch"] == pr["headRefName"] and moment(r["createdAt"]) <= moment(pr["mergedAt"])]
        if heads:
            last = max(heads, key=lambda r: r["createdAt"])
            push_to_merge.append(minutes(last["createdAt"], pr["mergedAt"]))
    rows.append({
        "metric": "Push → вливание, медиана",
        "value": fmt_minutes(statistics.median(push_to_merge)) if push_to_merge else "—",
        "target": "10 мин",
        "source": f"{len(push_to_merge)} вливаний с прогоном PR в окне",
    })

    to_site: list[float] = []
    pages_by_sha = defaultdict(list)
    for run in pages:
        pages_by_sha[run["headSha"]].append(run)
    for run in mains:
        rebuilt = [p for p in pages_by_sha.get(run["headSha"], []) if p["conclusion"] == "success"]
        if rebuilt:
            to_site.append(minutes(run["createdAt"], min(p["updatedAt"] for p in rebuilt)))
    rows.append({
        "metric": "Вливание → отчёт на сайте, медиана",
        "value": fmt_minutes(statistics.median(to_site)) if to_site else "—",
        "target": "15 мин",
        "source": f"{len(to_site)} push в main с пересборкой сайта",
    })

    green = sum(1 for r in mains if r["conclusion"] == "success")
    rows.append({
        "metric": "Зелёный main",
        "value": fmt_share(green, len(mains)),
        "target": "не ниже 98 %, каждый красный с причиной вне кода",
        "source": "push в main",
    })

    queue_green_shas = {r["headSha"] for r in queue if r["conclusion"] == "success"}
    false_green = [r for r in mains if r["conclusion"] == "failure" and r["headSha"] in queue_green_shas]
    rows.append({
        "metric": "Ложный зелёный: красный main при зелёной очереди",
        "value": str(len(false_green)),
        "target": "0",
        "source": "push в main против прогонов очереди по тому же коммиту",
    })

    if retry_trend:
        run_total = sum(int(b.get("data", {}).get("run", 0)) for b in retry_trend)
        retries = sum(int(b.get("data", {}).get("retry", 0)) for b in retry_trend)
        retry_value = "—" if run_total == 0 else f"{10000 * retries / run_total:.1f}".replace(".", ",")
    else:
        retry_value = "—"
    rows.append({
        "metric": "Повторы на 10 тысяч тестов",
        "value": retry_value,
        "target": "не выше 5",
        "source": "тренд повторов отчёта main",
    })

    by_sha = defaultdict(set)
    for r in prs:
        by_sha[r["headSha"]].add(r["conclusion"])
    flapping = sum(1 for outcomes in by_sha.values() if {"success", "failure"} <= outcomes)
    rows.append({
        "metric": "PR красный, затем зелёный без правки",
        "value": fmt_share(flapping, len(by_sha)),
        "target": "не выше 2 %",
        "source": "прогоны pull request одного и того же коммита",
    })

    dequeued = sum(1 for r in queue if r["conclusion"] in ("failure", "cancelled"))
    rows.append({
        "metric": "Выкидки из очереди",
        "value": fmt_share(dequeued, len(queue)),
        "target": "не выше 10 % постановок",
        "source": "прогоны merge_group без зелёного исхода",
    })

    stat = (summary or {}).get("statistic", {})
    rows.append({
        "metric": "Сломанные и упавшие тесты в отчёте main",
        "value": "—" if not stat else str(int(stat.get("failed", 0)) + int(stat.get("broken", 0))),
        "target": "не больше 3, у каждого задача с владельцем",
        "source": "сводка отчёта main",
    })

    per_merge = None
    if gh is not None and merged:
        cost_runs = prs + queue + mains
        average = runner_minutes(gh, repo, cost_runs)
        if average is not None:
            per_merge = average * len(cost_runs) / len(merged)
    rows.append({
        "metric": "Минуты раннеров на одно вливание",
        "value": "—" if per_merge is None else f"{per_merge:.0f}",
        "target": "40",
        "source": f"джобы последних {RUNNER_MINUTES_RUNS} прогонов PR, очереди и main, делённые на вливания",
    })

    days = max(1, WINDOW_DAYS)
    rows.append({
        "metric": "Вливаний в день",
        "value": f"{len(merged) / days:.1f}".replace(".", ","),
        "target": "40 при семи сессиях",
        "source": f"{len(merged)} вливаний за {days} дней",
    })
    return rows


def gather(repo: str, site: str, now: datetime | None = None, gh=gh_json, fetch=fetch_json, days: int = WINDOW_DAYS) -> dict:
    now = now or datetime.now(timezone.utc)
    since = now - timedelta(days=days)
    by_event = load_runs(gh, repo, since)
    merged = load_merged(gh, repo, since)
    retry_trend = fetch(f"{site}/allure/main/widgets/retry-trend.json") if site else None
    summary = fetch(f"{site}/allure/main/widgets/summary.json") if site else None
    return {
        "window_days": days,
        "since": since.strftime("%d.%m.%Y"),
        "until": now.strftime("%d.%m.%Y"),
        "metrics": compute(by_event, merged, retry_trend, summary, gh=gh, repo=repo),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", required=True)
    parser.add_argument("--site", default="")
    parser.add_argument("--days", type=int, default=WINDOW_DAYS)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    document = gather(args.repo, args.site, days=args.days)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    for row in document["metrics"]:
        print(f"{row['metric']}: {row['value']} (цель {row['target']})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
