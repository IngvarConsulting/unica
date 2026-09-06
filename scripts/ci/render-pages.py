#!/usr/bin/env python3
"""Render the published Unica page from one status document.

The page is static: every value is substituted at build time, so the browser
loads no script and the same bytes render offline, in an artifact and on Pages.
The status document is written next to the page for machines, which keeps the
human page and the machine answer from drifting apart.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import shutil
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PAGES = REPO_ROOT / "docs" / "pages"
MARK = REPO_ROOT / "docs" / "visual-kit" / "logos" / "unica-mark-blue.svg"
# Визуальный набор кладётся рядом с сайтом, а не ссылкой на репозиторий:
# ссылка на файл в гите ведёт на просмотрщик, а не на сам PDF.
VISUAL_KIT = REPO_ROOT / "docs" / "visual-kit" / "unica-visual-kit.pdf"
# Карточка для мессенджеров и соцсетей: без неё ссылка на сайт уходит в
# Telegram голой строкой. Баннер 1200×628 из кита подходит по пропорции.
SOCIAL_CARD = REPO_ROOT / "docs" / "visual-kit" / "ads" / "unica-ad-1200x628.png"
ASSETS = (
    (MARK, MARK.name),
    (VISUAL_KIT, VISUAL_KIT.name),
    (SOCIAL_CARD, "unica-social-card.png"),
)


def copy_assets(out: Path) -> None:
    (out / "assets").mkdir(parents=True, exist_ok=True)
    for source, name in ASSETS:
        shutil.copy2(source, out / "assets" / name)
STYLESHEET = PAGES / "site.css"
SCRIPT = PAGES / "site.js"
# Стили и скрипт лежат отдельными файлами, чтобы их правили в одном месте, но
# в страницу попадают телом: тогда страница открывается сама по себе — из
# артефакта, из письма, из локального файла.
INLINED = (
    ('<link rel="stylesheet" href="assets/site.css">', "style", STYLESHEET),
    ('<script src="assets/site.js"></script>', "script", SCRIPT),
)
PLACEHOLDER = re.compile(r"\{\{([a-z0-9_]+)\}\}")
SCENARIO_STATUS = Path(__file__).with_name("scenario-status.py")
REPEAT = re.compile(r"[ \t]*<!-- repeat: ([a-z_]+) -->\n(.*?)[ \t]*<!-- /repeat -->\n", re.S)


def inline_assets(page: str, name: str) -> str:
    """Заменить ссылки на стили и скрипт их телом.

    Отсутствие ссылки — не мелочь: в артефакт кладётся только знак, поэтому
    страница со ссылкой на `assets/site.css` уедет на сайт без оформления.
    Молчать об этом нельзя, поэтому пропавшая ссылка — отказ.
    """
    for marker, tag, source in INLINED:
        if marker not in page:
            raise SystemExit(f"{name}: нет ссылки на подстановку — {marker}")
        body = source.read_text(encoding="utf-8")
        page = page.replace(marker, f"<{tag}>\n{body}</{tag}>")
    return page


def placeholders(template: str) -> set[str]:
    return set(PLACEHOLDER.findall(template))


def corpus_status() -> dict[str, object]:
    """Данные каталога сценариев берутся из корпуса на каждой сборке.

    Их нельзя передать документом статуса: тогда страницу можно было бы
    опубликовать с прошлогодним каталогом, а корпус тем временем уехал.
    """
    spec = importlib.util.spec_from_file_location("scenario_status", SCENARIO_STATUS)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    corpus = json.loads(module.CORPUS.read_text(encoding="utf-8"))
    return module.build(corpus)


def expand_repeats(template: str, status: dict[str, object]) -> tuple[str, set[str]]:
    """Render a block once per item of its list.

    Карточек веток столько, сколько открытых линий, и число меняется само:
    линия закрывается через месяц после выхода следующей версии. Разметка при
    этом остаётся в шаблоне, а не переезжает в скрипт.
    """
    used: set[str] = set()

    def one(match: re.Match[str]) -> str:
        name, body = match.group(1), match.group(2)
        used.add(name)
        items = status.get(name)
        if not isinstance(items, list):
            raise SystemExit(f"status has no list for repeat block {name}")
        wanted = placeholders(body)
        rendered = []
        for index, item in enumerate(items):
            missing = sorted(wanted - set(item))
            unused = sorted(set(item) - wanted)
            if missing or unused:
                problem = []
                if missing:
                    problem.append(f"no value for {', '.join(missing)}")
                if unused:
                    problem.append(f"block never shows {', '.join(unused)}")
                raise SystemExit(f"{name}[{index}]: " + "; ".join(problem))
            rendered.append(PLACEHOLDER.sub(lambda m: str(item[m.group(1)]), body))
        return "".join(rendered)

    return REPEAT.sub(one, template), used


def render(template: str, status: dict[str, object]) -> str:
    """Substitute every placeholder, refusing a partial page.

    A missing key would ship `{{tests_failed}}` to a reader, and an unused key
    means the status document promises something the page never shows. Both are
    the same defect — the page and its source disagree — so both stop the run.
    """
    template, repeated = expand_repeats(template, status)
    wanted = placeholders(template)
    have = set(status) - repeated
    missing = sorted(wanted - have)
    if missing:
        raise SystemExit(f"status has no value for {', '.join(missing)}")
    return PLACEHOLDER.sub(lambda m: str(status[m.group(1)]), template)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--status", type=Path, required=True, help="JSON with one value per placeholder")
    parser.add_argument("--out", type=Path, required=True, help="directory to write the site into")
    parser.add_argument("--pages", type=Path, default=PAGES)
    args = parser.parse_args()

    status = json.loads(args.status.read_text(encoding="utf-8"))
    generated = corpus_status()
    overlap = sorted(set(status) & set(generated))
    if overlap:
        raise SystemExit(
            "status must not carry corpus values, they are generated: " + ", ".join(overlap)
        )
    status.update(generated)
    out = args.out
    copy_assets(out)

    written = []
    for template in sorted(args.pages.glob("*.html")):
        page = render(template.read_text(encoding="utf-8"), status)
        page = inline_assets(page, template.name)
        (out / template.name).write_text(page, encoding="utf-8")
        written.append(template.name)

    (out / "status.json").write_text(json.dumps(status, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("written: " + ", ".join(written))
    return 0


if __name__ == "__main__":
    sys.exit(main())
