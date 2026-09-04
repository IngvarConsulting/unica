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


def carry_history(results: Path, history: Path | None) -> int:
    """Положить историю прошлого прогона рядом с результатами.

    Без неё отчёт теряет тренды и перестаёт отличать перемежающийся тест от
    разового падения — то есть ровно то, ради чего история и публикуется.
    """
    if history is None or not history.is_dir():
        return 0
    target = results / "history"
    target.mkdir(parents=True, exist_ok=True)
    carried = 0
    for name in HISTORY_FILES:
        source = history / name
        if source.is_file():
            shutil.copy2(source, target / name)
            carried += 1
    return carried


def build_report(out: Path, args: argparse.Namespace) -> str:
    # Отчёт лежит под своей веткой: у каждой линии своя история, иначе тренды
    # перемешаются и «перемежающийся тест» окажется артефактом смешения.
    report = out / args.allure_path
    if args.allure_report:
        shutil.copytree(args.allure_report, report, dirs_exist_ok=True)
        return f"отчёт скопирован из {args.allure_report}"

    if not args.allure_results:
        return "отчёт не собирался: не передан ни --allure-results, ни --allure-report"

    results = Path(args.allure_results)
    if not results.is_dir():
        raise SystemExit(f"нет каталога результатов: {results}")
    carried = carry_history(results, Path(args.allure_history) if args.allure_history else None)

    executable = shutil.which(args.allure_command) or args.allure_command
    if not Path(executable).exists() and shutil.which(args.allure_command) is None:
        raise SystemExit(
            f"{args.allure_command} не найден: поставьте Allure CLI или передайте --allure-report"
        )
    subprocess.run(
        [executable, "generate", "--clean", "--output", str(report), str(results)],
        check=True,
    )
    return f"отчёт собран из {results}, перенесено файлов истории: {carried}"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--status", type=Path, required=True, help="значения страниц вне корпуса")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--allure-results", help="каталог allure-results прошедшего прогона")
    parser.add_argument("--allure-report", help="уже собранный отчёт")
    parser.add_argument("--allure-history", help="каталог history опубликованного прогона")
    parser.add_argument("--allure-command", default="allure")
    parser.add_argument("--allure-path", default="allure", help="куда класть отчёт внутри сайта")
    args = parser.parse_args()

    out = args.out
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    pages = render_pages(args.status, out)
    report = build_report(out, args)
    print("страницы: " + ", ".join(pages))
    print("allure: " + report)
    return 0


if __name__ == "__main__":
    sys.exit(main())
