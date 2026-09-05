#!/usr/bin/env python3
"""Собрать сайт целиком: страницы, данные корпуса и отчёт Allure.

Сайт публикуется одним артефактом, поэтому и собираться должен одной командой.
Разделение «страницы отдельно, отчёт отдельно» уже приводило бы к состоянию,
когда на странице есть ссылка на отчёт, а отчёта в артефакте нет.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import shutil
import subprocess
import sys
import tarfile
import urllib.error
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
RENDER = Path(__file__).with_name("render-pages.py")
HISTORY_FILES = (
    "history.json",
    "history-trend.json",
    "duration-trend.json",
    "retry-trend.json",
    "categories-trend.json",
)


def load(path: Path):
    spec = importlib.util.spec_from_file_location(path.stem.replace("-", "_"), path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def render_pages(status: Path, out: Path) -> list[str]:
    module = load(RENDER)
    status_document = json.loads(status.read_text(encoding="utf-8"))
    generated = module.corpus_status()
    overlap = sorted(set(status_document) & set(generated))
    if overlap:
        raise SystemExit("status carries generated corpus values: " + ", ".join(overlap))
    status_document.update(generated)

    module.copy_assets(out)
    written = []
    for template in sorted(module.PAGES.glob("*.html")):
        page = module.render(template.read_text(encoding="utf-8"), status_document)
        page = module.inline_assets(page, template.name)
        (out / template.name).write_text(page, encoding="utf-8")
        written.append(template.name)
    (out / "status.json").write_text(
        json.dumps(status_document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return written


def fetch(url: str, target: Path) -> bool:
    """Скачать файл с опубликованного сайта, если он там есть."""
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(response.read())
        return True
    except Exception:
        return False


def carry_history(results: Path, site: str | None, line: str) -> int:
    """Положить историю прошлого прогона рядом с результатами.

    Без неё отчёт теряет тренды и перестаёт отличать перемежающийся тест от
    разового падения — то есть ровно то, ради чего история и публикуется.
    """
    if not site:
        return 0
    target = results / "history"
    carried = 0
    for name in HISTORY_FILES:
        if fetch(f"{site}/allure/{line}/history/{name}", target / name):
            carried += 1
    return carried


TREND_FILES = tuple(name for name in HISTORY_FILES if name != "history.json")


def unwind_history(history: Path) -> int:
    """Снять вершину перенесённой истории перед пересборкой из сохранённых результатов.

    Вершина — тот самый прогон, что сейчас пересобирается: Allure положит его
    в историю заново, и без этого каждая пересборка сайта удваивала бы запуск
    в тренде и в истории каждого теста. Тест с единственным запуском убирается
    целиком — Allure заведёт его заново тем же запуском.
    """
    unwound = 0
    for name in TREND_FILES:
        path = history / name
        if not path.is_file():
            continue
        entries = json.loads(path.read_text(encoding="utf-8"))
        if entries:
            path.write_text(json.dumps(entries[1:], ensure_ascii=False), encoding="utf-8")
            unwound += 1
    path = history / "history.json"
    if path.is_file():
        kept = {}
        for key, entry in json.loads(path.read_text(encoding="utf-8")).items():
            items = list(entry.get("items", []))
            if len(items) < 2:
                continue
            top, rest = items[0], items[1:]
            statistic = dict(entry.get("statistic", {}))
            for field in (str(top.get("status", "")).lower(), "total"):
                if statistic.get(field, 0) > 0:
                    statistic[field] -= 1
            kept[key] = {**entry, "statistic": statistic, "items": rest}
        path.write_text(json.dumps(kept, ensure_ascii=False), encoding="utf-8")
        unwound += 1
    return unwound


SITE_PROFILES = ("main", "release", "large", "all")


def unpack(archive: Path, target: Path) -> Path:
    target.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "r:gz") as bundle:
        # Архив приезжает с сайта, а не из дерева. Сверять имена самому мало:
        # ссылка внутри архива уводит наружу уже после проверки. `data` —
        # фильтр самого tarfile: он отбрасывает и абсолютные пути, и `..`, и
        # ссылки за пределы каталога.
        bundle.extractall(target, filter="data")
    return target


def signature_of(results: Path | None) -> dict:
    path = results / "run.json" if results is not None else None
    return json.loads(path.read_text(encoding="utf-8")) if path is not None and path.is_file() else {}


def stored_results(site: str | None, line: str, work: Path) -> dict[str, tuple[Path, Path]]:
    """Хранимые результаты линии по профилям: профиль → (архив, каталог).

    Сайт хранит их у себя, поэтому срок жизни артефактов прогона ни на что не
    влияет: линия, которая сегодня не собиралась, всё равно покажет прошлый
    отчёт, а не исчезнет с сайта. Архив без профиля — из прежней раскладки —
    считается результатами `main`.
    """
    if not site:
        return {}
    found: dict[str, tuple[Path, Path]] = {}
    for profile in SITE_PROFILES:
        archive = work / f"{line}-{profile}.tar.gz"
        if fetch(f"{site}/data/{line}/results/{profile}.tar.gz", archive):
            found[profile] = (archive, unpack(archive, work / line / profile))
    if not found:
        archive = work / f"{line}-main.tar.gz"
        if fetch(f"{site}/data/{line}/results.tar.gz", archive):
            found["main"] = (archive, unpack(archive, work / line / "main"))
    return found


def merge_results(sources: list[Path], out: Path) -> int:
    """Объединить результаты: по тесту и раннеру побеждает поздняя запись.

    Отчёт линии — последнее известное состояние по всем ярусам: push обновляет
    свою часть, ночной `large` — свою, и ни один не стирает другую. Ключ —
    `historyId`, то есть имя теста и раннер; все записи выигравшего источника
    с этим ключом идут вместе, чтобы попытки повторов не потерялись. Метаданные
    — от первого источника, у которого они есть: свежий идёт первым.
    """
    winners: dict[str, tuple[int, list[Path]]] = {}
    for source in sources:
        groups: dict[str, tuple[int, list[Path]]] = {}
        for record in source.glob("*-result.json"):
            entry = json.loads(record.read_text(encoding="utf-8"))
            key = entry.get("historyId") or record.name
            stop, paths = groups.get(key, (0, []))
            groups[key] = (max(stop, int(entry.get("stop") or 0)), [*paths, record])
        for key, (stop, paths) in groups.items():
            if key not in winners or stop > winners[key][0]:
                winners[key] = (stop, paths)
    out.mkdir(parents=True, exist_ok=True)
    for _, paths in winners.values():
        for record in paths:
            shutil.copy2(record, out / record.name)
    for name in ("categories.json", "environment.properties", "executor.json", "run.json"):
        for source in sources:
            if (source / name).is_file():
                shutil.copy2(source / name, out / name)
                break
    return len(winners)


LARGE_PROFILES = ("large", "release")


def record_large_memory(data: Path, line: str, results: Path, fresh: bool, site: str | None) -> str:
    """Память ночного прогона: какую вершину линии `large` проверил последним.

    Ночной workflow читает её с сайта и пропускает линию, чья вершина на месте.
    Прогон по тегу пишет ту же память: после тега на вершине ночью — пропуск.
    Линия без такого прогона переносит память с сайта, иначе каждая
    пересборка страниц стирала бы её.
    """
    target = data / "profiles" / "large.json"
    signature = results / "run.json"
    if fresh and signature.is_file():
        run = json.loads(signature.read_text(encoding="utf-8"))
        if run.get("profile") in LARGE_PROFILES:
            target.parent.mkdir(parents=True, exist_ok=True)
            memory = {key: run.get(key, "") for key in ("sha", "at", "run_url", "run_id", "profile")}
            target.write_text(json.dumps(memory, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            return f"память large записана: {run.get('sha', '')[:7]}"
    if site and fetch(f"{site}/data/{line}/profiles/large.json", target):
        return "память large перенесена с сайта"
    return "памяти large нет"


def record_run(data: Path, line: str, fresh: bool, args: argparse.Namespace) -> None:
    """Подписать отчёт тем прогоном, который его дал.

    Линия, у которой сегодня прогона не было, пересобирается из результатов,
    лежащих на сайте. Подписать такой отчёт сборкой сайта — значит объявить
    вчерашние счётчики сегодняшними, поэтому метка либо приходит вместе со
    свежими результатами, либо переносится с сайта вместе с ними.
    """
    if not fresh:
        if args.site:
            fetch(f"{args.site}/data/{line}/run.json", data / "run.json")
        return
    record = {"sha": args.run_sha, "url": args.run_url, "at": args.run_at}
    (data / "run.json").write_text(
        json.dumps(record, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )


def publish_line(
    out: Path,
    line: str,
    results: Path,
    fresh: Path | None,
    profile: str | None,
    stored: dict[str, tuple[Path, Path]],
    args: argparse.Namespace,
) -> str:
    """Собрать отчёт линии из объединения и положить рядом сырые данные по профилям."""
    carried = carry_history(results, args.site, line)
    if fresh is None and carried:
        unwind_history(results / "history")
    if shutil.which(args.allure_command) is None:
        raise SystemExit(
            f"{args.allure_command} не найден: поставьте Allure CLI или уберите линию из сборки"
        )
    report = out / "allure" / line
    subprocess.run(
        [args.allure_command, "generate", "--clean", "--output", str(report), str(results)],
        check=True,
    )

    # Сырые результаты хранятся по профилям: свежий профиль перезаписывает
    # свой архив, остальные едут дальше как есть.
    data = out / "data" / line
    (data / "results").mkdir(parents=True, exist_ok=True)
    if fresh is not None:
        with tarfile.open(data / "results" / f"{profile}.tar.gz", "w:gz") as bundle:
            for item in sorted(fresh.iterdir()):
                if item.name != "history":
                    bundle.add(item, arcname=item.name)
    for other, (archive, _) in stored.items():
        shutil.copy2(archive, data / "results" / f"{other}.tar.gz")
    summary = report / "widgets" / "summary.json"
    if summary.is_file():
        shutil.copy2(summary, data / "summary.json")
    record_run(data, line, fresh is not None, args)
    memory = record_large_memory(data, line, fresh if fresh is not None else results, fresh is not None, args.site)
    return f"{line}: отчёт собран, файлов истории {carried}, результаты " + (
        f"свежие ({profile})" if fresh is not None else "с сайта, вершина истории снята"
    ) + f", профилей в объединении {len(stored) + (1 if fresh is not None else 0)}, {memory}"


def build_reports(out: Path, args: argparse.Namespace) -> list[str]:
    if not args.line:
        return ["отчёты не собирались: линии не переданы"]

    work = out.parent / ".lines"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)

    notes = []
    for entry in args.line:
        line, _, fresh_dir = entry.partition("=")
        fresh = Path(fresh_dir) if fresh_dir else None
        if fresh is not None and not fresh.is_dir():
            notes.append(f"{line}: каталога свежих результатов {fresh} нет")
            continue
        stored = stored_results(args.site, line, work)
        profile = (signature_of(fresh).get("profile") or "main") if fresh is not None else None
        others = {name: pair for name, pair in stored.items() if name != profile}
        if fresh is None and not others:
            notes.append(f"{line}: результатов нет — ни свежих, ни на сайте")
            continue
        # Хранимые — от новых к старым: метаданные пересборки идут от последнего прогона.
        ordered = sorted(others.values(), key=lambda pair: signature_of(pair[1]).get("at", ""), reverse=True)
        sources = ([fresh] if fresh is not None else []) + [directory for _, directory in ordered]
        merged = work / line / "merged"
        records = merge_results(sources, merged)
        notes.append(publish_line(out, line, merged, fresh, profile, others, args) + f", записей {records}")
    return notes


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--status", type=Path, required=True, help="значения страниц вне корпуса")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument(
        "--line",
        action="append",
        default=[],
        metavar="ИМЯ[=КАТАЛОГ]",
        help="линия сайта; с каталогом — свежие результаты, без него — прошлые с сайта",
    )
    parser.add_argument(
        "--site", help="адрес опубликованного сайта: оттуда берутся история и прошлые результаты"
    )
    parser.add_argument("--run-sha", default="", help="коммит прогона, давшего свежие результаты")
    parser.add_argument("--run-url", default="", help="адрес этого прогона")
    parser.add_argument("--run-at", default="", help="время этого прогона в ISO-8601")
    parser.add_argument("--allure-command", default="allure")
    parser.add_argument(
        "--pages-only",
        action="store_true",
        help="перерисовать страницы поверх собранного сайта, не трогая отчёты",
    )
    args = parser.parse_args()

    out = args.out
    if not args.pages_only:
        if out.exists():
            shutil.rmtree(out)
        out.mkdir(parents=True)

    pages = render_pages(args.status, out)
    print("страницы: " + ", ".join(pages))
    if args.pages_only:
        return 0
    for note in build_reports(out, args):
        print("allure: " + note)
    return 0


if __name__ == "__main__":
    sys.exit(main())
