#!/usr/bin/env python3
"""Собрать документ статуса сайта из релизов, веток и отчёта прогона.

Ни одно значение здесь не пишется руками: версия приходит из релиза, состав
открытых линий — из правила закрытия, счётчики тестов — из сводки собранного
отчёта. Страница не может разойтись с тем, что опубликовано.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

LINE_BRANCH = re.compile(r"\Arelease-v(\d+)\.(\d+)\Z")
# Линия патчей живёт ещё месяц после того, как вышла следующая minor.
LINE_GRACE = timedelta(days=30)


def gh(repo: str, path: str) -> list:
    """Все страницы ответа одним списком.

    `--slurp` возвращает массив страниц, поэтому его достаточно развернуть —
    склейка потока JSON регулярным выражением ломается на первой же кавычке
    внутри строки.
    """
    result = subprocess.run(
        ["gh", "api", f"repos/{repo}/{path}", "--paginate", "--slurp"],
        capture_output=True,
        text=True,
        check=True,
    )
    pages = json.loads(result.stdout)
    merged: list = []
    for page in pages:
        merged.extend(page if isinstance(page, list) else [page])
    return merged


def moment(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def human(value: datetime) -> str:
    return value.astimezone(timezone.utc).strftime("%d.%m.%Y")


def open_lines(repo: str, now: datetime) -> list[str]:
    """Какие линии патчей ещё открыты.

    Линия закрывается через месяц после первого релиза следующей minor: до
    этого момента патч может выйти и там, и там.
    """
    releases = [r for r in gh(repo, "releases?per_page=100") if not r["draft"]]
    first_of_minor: dict[tuple[int, int], datetime] = {}
    for release in releases:
        found = re.match(r"\Av(\d+)\.(\d+)\.(\d+)", release["tag_name"])
        if not found:
            continue
        key = (int(found.group(1)), int(found.group(2)))
        published = moment(release["published_at"])
        first_of_minor[key] = min(first_of_minor.get(key, published), published)

    lines = []
    for branch in gh(repo, "branches?per_page=100"):
        found = LINE_BRANCH.match(branch["name"])
        if not found:
            continue
        key = (int(found.group(1)), int(found.group(2)))
        successor = min(
            (published for minor, published in first_of_minor.items() if minor > key),
            default=None,
        )
        if successor is not None and now - successor > LINE_GRACE:
            continue
        lines.append(branch["name"])
    return sorted(lines)


def summary_counts(path: Path | None) -> dict[str, str] | None:
    """Счётчики берутся из сводки собранного отчёта, а не из воздуха."""
    if path is None or not path.is_file():
        return None
    statistic = json.loads(path.read_text(encoding="utf-8")).get("statistic", {})
    total = statistic.get("total", 0)
    return {
        "tests_total": f"{total}",
        "tests_passed": f"{statistic.get('passed', 0)}",
        "tests_failed": f"{statistic.get('failed', 0) + statistic.get('broken', 0)}",
        "tests_skipped": f"{statistic.get('skipped', 0)}",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", ""))
    parser.add_argument("--branch", default="main")
    parser.add_argument("--sha", default=os.environ.get("GITHUB_SHA", ""))
    parser.add_argument("--run-url", default="")
    parser.add_argument("--run-at", default="", help="время прогона в ISO-8601")
    parser.add_argument("--allure-summary", type=Path, help="widgets/summary.json собранного отчёта")
    parser.add_argument("--report-url", default="allure/main/")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    if not args.repo:
        raise SystemExit("нужен --repo или GITHUB_REPOSITORY")
    now = datetime.now(timezone.utc)

    releases = [r for r in gh(args.repo, "releases?per_page=100") if not r["draft"]]
    published = [r for r in releases if not r["prerelease"]]
    prereleases = [r for r in releases if r["prerelease"]]
    if not published:
        raise SystemExit("у репозитория нет опубликованных релизов")
    latest = max(published, key=lambda r: moment(r["published_at"]))

    status: dict[str, object] = {
        "version": latest["tag_name"],
        "version_date": human(moment(latest["published_at"])),
        "version_url": latest["html_url"],
        "generated_at": human(now) + now.astimezone(timezone.utc).strftime(" %H:%M"),
        "generated_sha": (args.sha or "")[:7] or "—",
        "generated_url": args.run_url or f"https://github.com/{args.repo}/actions",
    }

    # Пререлиз показывается только пока он впереди опубликованной версии:
    # прошлогодний rc уже ничего не готовит.
    newest = max(prereleases, key=lambda r: moment(r["published_at"]), default=None)
    if newest and moment(newest["published_at"]) > moment(latest["published_at"]):
        status["prerelease"] = newest["tag_name"]
        status["prerelease_note"] = "опубликован " + human(moment(newest["published_at"]))
    else:
        status["prerelease"] = "Планируется"
        status["prerelease_note"] = "сборка перед публикацией"

    counts = summary_counts(args.allure_summary)
    run_at = human(moment(args.run_at)) if args.run_at else human(now)
    tested, plain = [], []
    for line in [args.branch, *open_lines(args.repo, now)]:
        if line == args.branch and counts:
            tested.append(
                {
                    "line": line,
                    "build_sha": (args.sha or "")[:7] or "—",
                    "build_date": run_at,
                    "build_url": status["generated_url"],
                    "report_url": args.report_url,
                    **counts,
                }
            )
        elif line != args.branch or not counts:
            plain.append(
                {
                    "line": line,
                    "build_state": "Нет прогонов",
                    "build_note": "отчёт появится, когда линия начнёт его собирать",
                }
            )
    status["tested_lines"] = tested
    status["plain_lines"] = plain

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(status, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"written: {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
