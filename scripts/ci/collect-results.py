#!/usr/bin/env python3
"""Сложить артефакты прогона-источника в результаты линии для сайта.

Джобы оставляют `results-<джоба>` с `allure-results` и подписью `run.json`, а
Rust-джобы ещё и `plan-rust-<раннер>` — план, выгруженный до тестов. Здесь
всё это складывается в один каталог на линию, и по плану дописывается то,
до чего раннер не дошёл: такой тест попадает в отчёт `skipped` с причиной
«раннер не дошёл», а не исчезает из истории.

Линия берётся из подписи прогона, а не из события: у прогона по расписанию
`head_branch` всегда `main`, даже когда он проверяет релизную линию.
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


def result_dirs(artifacts: Path) -> list[Path]:
    return sorted(path for path in artifacts.glob("results-*") if path.is_dir())


def plan_dirs(artifacts: Path) -> list[Path]:
    return sorted(path for path in artifacts.glob("plan-rust-*") if path.is_dir())


def signature(dirs: list[Path]) -> dict:
    """Подпись прогона — любая из джоб: коммит, ссылка и ветка у них общие."""
    for path in dirs:
        run = path / "run.json"
        if run.is_file():
            return load_json(run)
    return {}


def job_conclusions(jobs: Path | None) -> dict[str, str]:
    if jobs is None or not jobs.is_file():
        return {}
    listed = load_json(jobs)
    return {job["name"]: (job.get("conclusion") or job.get("status") or "unknown") for job in listed.get("jobs", [])}


def copy_results(dirs: list[Path], out: Path) -> tuple[int, dict[str, set[str]]]:
    """Скопировать записи; вернуть счёт и «раннер → полные имена Rust»."""
    seen: dict[str, set[str]] = {}
    count = 0
    for path in dirs:
        for record in path.glob("*-result.json"):
            shutil.copy2(record, out / record.name)
            count += 1
            entry = load_json(record)
            if label(entry, "language") == "rust":
                seen.setdefault(label(entry, "host") or "", set()).add(entry["fullName"])
    return count, seen


def fill_gaps(plans: list[Path], seen: dict[str, set[str]], out: Path, run: dict, conclusions: dict[str, str]) -> int:
    """Тест из плана без результата — раннер не дошёл. Записать как `skipped`."""
    filled = 0
    for plan_dir in plans:
        runner = plan_dir.name.removeprefix("plan-rust-")
        planned = load_json(plan_dir / "plan.json")
        have = seen.get(runner, set())
        job = f"Rust tests ({runner})"
        conclusion = conclusions.get(job) or conclusions.get(f"{job} primary") or "unknown"
        for case in planned:
            full_name = f"{case['binary']}::{case['name']}"
            if full_name in have:
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
                    labels=allure_results.rust_labels(case["binary"], case["name"], run.get("profile", "all")),
                    tags=(run.get("profile", "all"), "infrastructure"),
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


def collect(artifacts: Path, out_root: Path, jobs: Path | None, fallback_line: str, site: str) -> tuple[str, dict]:
    dirs = result_dirs(artifacts)
    if not dirs:
        raise SystemExit(f"в {artifacts} нет ни одного results-*: складывать нечего")
    run = signature(dirs)
    line = run.get("ref") or fallback_line
    if not line:
        raise SystemExit("линия не определена: ни подписи прогона, ни --fallback-line")
    # Линия — имя ветки без разделителей: `main` или `release-vX.Y`. Ссылка
    # вида `722/merge` приходит с pull request, которого сайт не публикует, и
    # каталог с косой чертой внутри выдал бы вложенный путь вместо линии.
    if "/" in line or line.startswith("."):
        raise SystemExit(f"{line!r} — не имя линии; сайт публикует только ветки без разделителей")
    out = out_root / line
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    copied, seen = copy_results(dirs, out)
    filled = fill_gaps(plan_dirs(artifacts), seen, out, run, job_conclusions(jobs))
    runners = sorted({(load_json(p / "run.json").get("runner") or "") for p in dirs if (p / "run.json").is_file()})
    write_metadata(out, run, line, [r for r in runners if r], site)
    return line, {"copied": copied, "filled": filled, "runners": runners}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--artifacts", type=Path, required=True, help="каталог со скачанными артефактами")
    parser.add_argument("--out", type=Path, required=True, help="куда сложить: <out>/<линия>/")
    parser.add_argument("--jobs", type=Path, default=None, help="ответ API со списком джоб прогона")
    parser.add_argument("--fallback-line", default="", help="линия, если подписи нет")
    parser.add_argument("--site", default="", help="адрес сайта для ссылки на отчёт")
    args = parser.parse_args(argv)

    line, stats = collect(args.artifacts, args.out, args.jobs, args.fallback_line, args.site)
    print(f"{line}: записей {stats['copied']}, дописано недошедших {stats['filled']}, раннеры {', '.join(r for r in stats['runners'] if r)}", file=sys.stderr)
    print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
