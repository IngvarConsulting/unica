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
import urllib.request
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
    endpoint = f"repos/{repo}/{path}" if path else f"repos/{repo}"
    result = subprocess.run(
        ["gh", "api", endpoint, "--paginate", "--slurp"],
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


def site_lines(branch: str, repo: str, now: datetime) -> list[str]:
    """Линия прогона и открытые релизные линии, каждая по одному разу.

    Прогон релизной линии приходит с самой линии, поэтому её имя стоит и в
    `--branch`, и в списке открытых. Без свёртки отчёт этой линии собирался бы
    дважды, а на странице появлялась бы вторая такая же строка.
    """
    return list(dict.fromkeys([branch, *open_lines(repo, now)]))


def line_run(path: Path | None, repo: str) -> dict[str, str]:
    """Чей прогон дал этот отчёт.

    Отчёт линии пересобирается из результатов, которые хранит сайт, — он тогда
    показывает прошлый прогон, а не тот, что собирает сайт сейчас. Подпись
    берётся из метки, сохранённой рядом с результатами, поэтому вчерашние
    счётчики не выдаются за сегодняшние.
    """
    record = json.loads(path.read_text(encoding="utf-8")) if path and path.is_file() else {}
    at = record.get("at")
    return {
        "sha": (record.get("sha") or "")[:7] or "—",
        "date": human(moment(at)) if at else "—",
        "url": record.get("url") or f"https://github.com/{repo}/actions",
    }


def github_stars(repo: str) -> str:
    """Звёзд у репозитория на момент сборки."""
    try:
        return str(gh(repo, "")[0]["stargazers_count"])
    except Exception:
        return "—"


def telegram_members(chat: str) -> str:
    """Сколько человек в группе.

    Из браузера это число не прочитать: `t.me` не отдаёт CORS-заголовков, а у
    Bot API нет входа без токена, и класть токен в страницу нельзя. Поэтому
    число печатается при сборке и живёт до следующей.
    """
    try:
        with urllib.request.urlopen(f"https://t.me/{chat}", timeout=20) as response:
            page = response.read().decode("utf-8", "replace")
    except Exception:
        return "—"
    found = re.search(r'tgme_page_extra">([\d\s \xa0]+)\s+(?:members|subscribers)', page)
    return re.sub(r"[\s \xa0]", "", found.group(1)) if found else "—"


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
    parser.add_argument("--allure-summary", type=Path, help="widgets/summary.json собранного отчёта")
    parser.add_argument("--summaries", type=Path, help="каталог data/ собранного сайта: сводка на линию")
    parser.add_argument("--print-lines", action="store_true", help="напечатать открытые линии и выйти")
    parser.add_argument("--telegram", default="unica_ai", help="публичная группа Telegram")
    parser.add_argument("--report-url", default="allure/main/")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    if not args.repo:
        raise SystemExit("нужен --repo или GITHUB_REPOSITORY")
    now = datetime.now(timezone.utc)

    if args.print_lines:
        print("\n".join(site_lines(args.branch, args.repo, now)))
        return 0

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
        # Страницу собрал либо прогон, либо человек у себя. Во втором случае
        # коммита нет, и подписывать её прочерком — значит промолчать: пусть
        # прямо говорит `local` и ведёт в репозиторий, а не на прогон, которого
        # не было.
        "generated_sha": (args.sha or "")[:7] or "local",
        "generated_url": args.run_url
        or (
            f"https://github.com/{args.repo}/commit/{args.sha}"
            if args.sha
            else f"https://github.com/{args.repo}"
        ),
        "github_stars": github_stars(args.repo),
        "telegram_members": telegram_members(args.telegram),
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

    tested, plain = [], []
    for line in site_lines(args.branch, args.repo, now):
        data = args.summaries / line if args.summaries else None
        counts = summary_counts(data / "summary.json" if data else args.allure_summary)
        if counts:
            run = line_run(data / "run.json" if data else None, args.repo)
            tested.append(
                {
                    "line": line,
                    "build_sha": run["sha"],
                    "build_date": run["date"],
                    "build_url": run["url"],
                    "report_url": f"allure/{line}/",
                    **counts,
                }
            )
        else:
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
