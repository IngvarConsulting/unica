#!/usr/bin/env python3
"""Сложить артефакты прогона-источника в результаты линий для сайта.

Джобы оставляют `results-*` с `allure-results` и подписью `run.json`, а
Rust-джобы ещё и `plan-*` — план, выгруженный до тестов, с той же подписью.
Здесь всё это складывается по линиям, и по плану дописывается то, до чего
раннер не дошёл: такой тест попадает в отчёт `skipped` с причиной «раннер не
дошёл», а не исчезает из истории.

Линия берётся из подписи каждого артефакта, а не из события: у прогона по
расписанию `head_branch` всегда `main`, даже когда он проверяет релизную
линию, а ночной прогон несёт несколько линий сразу.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import allure_results  # noqa: E402

CATEGORIES = [
    {"name": "Инфраструктура: раннер не дошёл", "matchedStatuses": ["skipped"], "messageRegex": "раннер не дошёл.*"},
    {"name": "Отключены автором", "matchedStatuses": ["skipped"], "messageRegex": "отключён автором.*"},
    {"name": "Не в этом ярусе", "matchedStatuses": ["skipped"], "messageRegex": "cadence:.*"},
    {"name": "Поломка теста, а не продукта", "matchedStatuses": ["broken"]},
    {"name": "Дефект продукта", "matchedStatuses": ["failed"]},
]


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def label(entry: dict, name: str) -> str | None:
    for item in entry.get("labels", []):
        if item.get("name") == name:
            return item.get("value")
    return None


def signed_dirs(artifacts: Path, prefix: str) -> list[tuple[Path, dict]]:
    """Каталоги артефактов с подписью прогона; без подписи каталог не читается."""
    found = []
    for path in sorted(artifacts.glob(f"{prefix}*")):
        if path.is_dir() and (path / "run.json").is_file():
            found.append((path, load_json(path / "run.json")))
    return found


def line_of(run: dict, fallback: str) -> str:
    line = run.get("ref") or fallback
    if not line:
        raise SystemExit("линия не определена: ни подписи прогона, ни --fallback-line")
    # Линия — имя ветки без разделителей: `main` или `release-vX.Y`. Ссылка
    # вида `722/merge` приходит с pull request, которого сайт не публикует, и
    # каталог с косой чертой внутри выдал бы вложенный путь вместо линии.
    if "/" in line or line.startswith("."):
        raise SystemExit(f"{line!r} — не имя линии; сайт публикует только ветки без разделителей")
    return line


def job_conclusions(jobs: Path | None) -> dict[str, str]:
    if jobs is None or not jobs.is_file():
        return {}
    listed = load_json(jobs)
    return {job["name"]: (job.get("conclusion") or job.get("status") or "unknown") for job in listed.get("jobs", [])}


def copy_results(path: Path, out: Path) -> tuple[int, set[str]]:
    """Скопировать записи; вернуть счёт и полные имена Rust-тестов."""
    seen: set[str] = set()
    count = 0
    for record in path.glob("*-result.json"):
        shutil.copy2(record, out / record.name)
        count += 1
        entry = load_json(record)
        if label(entry, "language") == "rust":
            seen.add(entry["fullName"])
    return count, seen


def fill_gaps(plan_dir: Path, run: dict, seen: set[str], out: Path, conclusions: dict[str, str]) -> int:
    """Тест из плана без результата — раннер не дошёл. Записать как `skipped`."""
    runner = run.get("runner", "")
    profile = run.get("profile", "all")
    job = f"Rust tests ({runner})"
    conclusion = conclusions.get(job) or conclusions.get(f"{job} primary") or "unknown"
    filled = 0
    for case in load_json(plan_dir / "plan.json"):
        full_name = f"{case['binary']}::{case['name']}"
        if full_name in seen:
            continue
        message = f"раннер не дошёл: {job} · {conclusion}"
        if run.get("run_url"):
            message += f" · {run['run_url']}"
        allure_results.write(
            out,
            allure_results.record(
                name=case["name"],
                full_name=full_name,
                status="skipped",
                runner=runner,
                labels=allure_results.rust_labels(case["binary"], case["name"], profile),
                tags=(profile, "infrastructure"),
                message=message,
            ),
        )
        filled += 1
    return filled


def properties_line(key: str, value: str) -> str:
    """Строка Java Properties: файл читается как ISO-8859-1, всё вне ASCII — `\\uXXXX`.

    Иначе кириллица в виджете окружения превращается в кракозябры — так и
    вышло на первом отчёте.
    """

    def escape(text: str) -> str:
        return "".join(c if ord(c) < 128 else f"\\u{ord(c):04x}" for c in text)

    return f"{escape(key)}={escape(value)}\n"


def write_metadata(out: Path, run: dict, line: str, runners: list[str], site: str) -> None:
    (out / "categories.json").write_text(json.dumps(CATEGORIES, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    environment = {
        "Ветка": line,
        "Коммит": run.get("sha", ""),
        "Прогон": run.get("run_url", ""),
        "Попытка": run.get("run_attempt", ""),
        "Профиль": run.get("profile", ""),
        "Раннеры": ", ".join(runners),
    }
    (out / "environment.properties").write_text(
        "".join(properties_line(key, value) for key, value in environment.items() if value), encoding="ascii"
    )
    executor = {
        "name": "GitHub Actions",
        "type": "github",
        "url": run.get("run_url", ""),
        "buildOrder": int(run["run_id"]) if str(run.get("run_id", "")).isdigit() else 0,
        "buildName": f"{line} · {run.get('sha', '')[:7]}",
        "buildUrl": run.get("run_url", ""),
        # Без завершающей косой черты: Allure сам дописывает `/#testresult/…`.
        "reportUrl": f"{site}/allure/{line}" if site else "",
    }
    (out / "executor.json").write_text(json.dumps(executor, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    # Подпись линии едет дальше: сборка сайта пишет по ней память о прогоне.
    (out / "run.json").write_text(json.dumps({**run, "ref": line}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def collect(artifacts: Path, out_root: Path, jobs: Path | None, fallback_line: str, site: str) -> dict[str, dict]:
    """Сложить по линиям. Возвращает «линия → счёт записей, недошедших, раннеры»."""
    results = signed_dirs(artifacts, "results-")
    if not results:
        raise SystemExit(f"в {artifacts} нет ни одного results-* с подписью: складывать нечего")
    conclusions = job_conclusions(jobs)
    by_line: dict[str, dict] = {}
    seen: dict[tuple[str, str], set[str]] = {}
    for path, run in results:
        line = line_of(run, fallback_line)
        out = out_root / line
        if line not in by_line:
            if out.exists():
                shutil.rmtree(out)
            out.mkdir(parents=True)
            by_line[line] = {"copied": 0, "filled": 0, "runners": set(), "run": run}
        copied, names = copy_results(path, out)
        by_line[line]["copied"] += copied
        if run.get("runner"):
            by_line[line]["runners"].add(run["runner"])
        seen.setdefault((line, run.get("runner", "")), set()).update(names)
    for plan_dir, run in signed_dirs(artifacts, "plan-"):
        line = line_of(run, fallback_line)
        if line not in by_line:
            continue
        have = seen.get((line, run.get("runner", "")), set())
        by_line[line]["filled"] += fill_gaps(plan_dir, run, have, out_root / line, conclusions)
    for line, stats in by_line.items():
        stats["runners"] = sorted(stats["runners"])
        write_metadata(out_root / line, stats.pop("run"), line, stats["runners"], site)
    return by_line


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--artifacts", type=Path, required=True, help="каталог со скачанными артефактами")
    parser.add_argument("--out", type=Path, required=True, help="куда сложить: <out>/<линия>/")
    parser.add_argument("--jobs", type=Path, default=None, help="ответ API со списком джоб прогона")
    parser.add_argument("--fallback-line", default="", help="линия, если подписи нет")
    parser.add_argument("--site", default="", help="адрес сайта для ссылки на отчёт")
    args = parser.parse_args(argv)

    lines = collect(args.artifacts, args.out, args.jobs, args.fallback_line, args.site)
    for line, stats in lines.items():
        print(
            f"{line}: записей {stats['copied']}, дописано недошедших {stats['filled']}, раннеры {', '.join(stats['runners'])}",
            file=sys.stderr,
        )
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
