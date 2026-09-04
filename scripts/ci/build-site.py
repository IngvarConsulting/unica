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

    styles = module.STYLESHEET.read_text(encoding="utf-8")
    (out / "assets").mkdir(parents=True, exist_ok=True)
    written = []
    for template in sorted(module.PAGES.glob("*.html")):
        page = module.render(template.read_text(encoding="utf-8"), status_document)
        page = page.replace(
            '<link rel="stylesheet" href="assets/site.css">',
            "<style>\n" + styles + "</style>",
        )
        (out / template.name).write_text(page, encoding="utf-8")
        written.append(template.name)
    (out / "status.json").write_text(
        json.dumps(status_document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    shutil.copy2(module.MARK, out / "assets" / module.MARK.name)
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


def previous_results(site: str | None, line: str, work: Path) -> Path | None:
    """Взять результаты последнего опубликованного прогона линии с сайта.

    Сайт хранит их у себя, поэтому срок жизни артефактов прогона ни на что не
    влияет: линия, которая сегодня не собиралась, всё равно покажет прошлый
    отчёт, а не исчезнет с сайта.
    """
    if not site:
        return None
    archive = work / f"{line}.tar.gz"
    if not fetch(f"{site}/data/{line}/results.tar.gz", archive):
        return None
    unpacked = work / line / "results"
    unpacked.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "r:gz") as bundle:
        root = unpacked.resolve()
        for member in bundle.getmembers():
            # Архив приезжает с сайта, а не из дерева: наружу его не пускаем.
            if not str((unpacked / member.name).resolve()).startswith(str(root)):
                raise SystemExit(f"архив линии {line} ведёт наружу: {member.name}")
        bundle.extractall(unpacked)
    return unpacked


def publish_line(out: Path, line: str, results: Path, args: argparse.Namespace) -> str:
    """Собрать отчёт линии и положить рядом сырые данные для следующего раза."""
    carried = carry_history(results, args.site, line)
    if shutil.which(args.allure_command) is None:
        raise SystemExit(
            f"{args.allure_command} не найден: поставьте Allure CLI или уберите линию из сборки"
        )
    report = out / "allure" / line
    subprocess.run(
        [args.allure_command, "generate", "--clean", "--output", str(report), str(results)],
        check=True,
    )

    data = out / "data" / line
    data.mkdir(parents=True, exist_ok=True)
    with tarfile.open(data / "results.tar.gz", "w:gz") as bundle:
        for item in sorted(results.iterdir()):
            bundle.add(item, arcname=item.name)
    summary = report / "widgets" / "summary.json"
    if summary.is_file():
        shutil.copy2(summary, data / "summary.json")
    return f"{line}: отчёт собран, файлов истории {carried}"


def build_reports(out: Path, args: argparse.Namespace) -> list[str]:
    if not args.line:
        return ["отчёты не собирались: линии не переданы"]

    work = out.parent / ".lines"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)

    notes = []
    for entry in args.line:
        line, _, fresh = entry.partition("=")
        results = Path(fresh) if fresh else previous_results(args.site, line, work)
        if results is None or not results.is_dir():
            notes.append(f"{line}: результатов нет — ни свежих, ни на сайте")
            continue
        notes.append(publish_line(out, line, results, args))
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
