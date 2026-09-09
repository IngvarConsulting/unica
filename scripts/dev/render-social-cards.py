#!/usr/bin/env python3
"""Render one social card per published page from the page's own copy.

A link to the site is opened in a feed before it is opened in a browser:
Telegram, VK, X and LinkedIn all show `og:image`, and a link without one goes
out as a bare string. The cards live next to the pages and are rendered here,
not written by hand, so the card and the page cannot say different things —
the eyebrow, the heading with its accent word and the one-line description are
read out of the page itself.

Why the PNG is committed instead of being built on the site workflow: the card
is typeset by the browser with the same system font stack the site asks for,
and that stack resolves differently on the runner than on a designer's Mac.
A card rebuilt on Ubuntu would silently change typeface on every deploy. So
the render happens once, on a machine with the fonts, and the result is a
tracked artifact — the same arrangement the visual kit already uses.

    python scripts/dev/render-social-cards.py            # rewrite the cards
    python scripts/dev/render-social-cards.py --check    # fail if they drift

`--check` re-renders into a temporary directory and compares bytes. It is a
developer's check, not a gate: CI has neither the browser nor the fonts, and
the guard that does run there — `tests/ci/test_social_cards.py` — checks the
wiring instead: every page points at a card that exists and is the size the
page declares.
"""

from __future__ import annotations

import argparse
import html
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PAGES = REPO_ROOT / "docs" / "pages"
CARDS = PAGES / "og"
TEMPLATE = CARDS / "card.html"

WIDTH, HEIGHT = 1200, 630

# Каждое поле карточки берётся из страницы, а не пишется рядом: копия, которую
# развели руками, расходится на первой же правке заголовка.
EYEBROW = re.compile(r'<p class="eyebrow">(.*?)</p>', re.S)
HEADING = re.compile(r"<h1>(.*?)</h1>", re.S)
DESCRIPTION = re.compile(r'<meta name="description" content="([^"]*)">')
PLACEHOLDER = re.compile(r"\{\{([a-z]+)\}\}")
# В заголовке разрешён ровно один тег — `<span>` вокруг акцентного слова.
# Всё остальное на карточке отрисовалось бы разметкой, поэтому это отказ.
HEADING_MARKUP = re.compile(r"</?(?!span\b)[a-z]")
# Единственное поле-разметка. Остальные поля карточки — текст: со страницы они
# снимаются распакованными (`&amp;` → `&`) и в шаблон входят экранированными
# обратно. Без второго шага описание, где автор страницы написал `&lt;b&gt;`,
# стало бы на карточке настоящим тегом: слово пропало бы, а вёрстка поехала.
MARKUP_FIELDS = frozenset({"heading"})

CHROME_ENV = "CHROME"
CHROME_PATHS = (
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "google-chrome",
    "chromium",
    "chromium-browser",
)


def find_chrome() -> str:
    """Локатор браузера: переменная окружения, затем известные места."""
    named = os.environ.get(CHROME_ENV)
    if named:
        if not (Path(named).is_file() or shutil.which(named)):
            raise SystemExit(f"{CHROME_ENV}={named}: такого браузера нет")
        return named
    for candidate in CHROME_PATHS:
        if Path(candidate).is_file():
            return candidate
        found = shutil.which(candidate)
        if found:
            return found
    raise SystemExit(
        "не нашёл Chrome или Chromium; укажите путь через переменную "
        f"{CHROME_ENV}=<путь к браузеру>"
    )


def one(pattern: re.Pattern[str], page: str, name: str, field: str) -> str:
    """Единственное совпадение или отказ: карточка без поля — не карточка."""
    found = pattern.search(page)
    if found is None:
        raise SystemExit(f"{name}: на странице нет поля {field}")
    return found.group(1).strip()


def collapse(text: str) -> str:
    """Свернуть перенос строки в пробел.

    Разметка страницы переносит длинные строки по ширине файла, и в вёрстке
    это пробел. В карточке перенос остался бы двойным пробелом посреди фразы.
    """
    return " ".join(text.split())


def without_heading(lede: str, heading: str) -> str:
    """Убрать из описания повтор заголовка.

    Описание страницы пишется для поисковой выдачи, где рядом с заголовком оно
    стоит уместно: «Как устроена Unica: кто приходит по MCP…». На карточке
    заголовок уже набран крупно, и то же начало во второй строке читается как
    заикание. Отрезается только буквальный повтор в начале и разделитель за
    ним; всё остальное описание остаётся как есть.
    """
    for separator in (": ", " — ", ". "):
        prefix = heading + separator
        if lede.lower().startswith(prefix.lower()):
            rest = lede[len(prefix):]
            return rest[:1].upper() + rest[1:]
    return lede


def card_fields(page_path: Path) -> dict[str, str]:
    page = page_path.read_text(encoding="utf-8")
    name = page_path.name
    heading = collapse(one(HEADING, page, name, "<h1>"))
    if HEADING_MARKUP.search(heading):
        raise SystemExit(f"{name}: в <h1> есть разметка кроме <span>: {heading}")
    title = html.unescape(re.sub(r"<[^>]+>", "", heading))
    lede = collapse(one(DESCRIPTION, page, name, 'meta name="description"'))
    return {
        "title": title,
        "eyebrow": html.unescape(collapse(one(EYEBROW, page, name, 'class="eyebrow"'))),
        "heading": heading,
        "lede": without_heading(html.unescape(lede), title),
    }


def render_html(template: str, fields: dict[str, str]) -> str:
    wanted = set(PLACEHOLDER.findall(template))
    missing = sorted(wanted - set(fields))
    unused = sorted(set(fields) - wanted)
    if missing or unused:
        problem = []
        if missing:
            problem.append("шаблон просит " + ", ".join(missing))
        if unused:
            problem.append("карточка не показывает " + ", ".join(unused))
        raise SystemExit(f"{TEMPLATE.name}: " + "; ".join(problem))

    def value(match: re.Match[str]) -> str:
        name = match.group(1)
        return fields[name] if name in MARKUP_FIELDS else html.escape(fields[name])

    return PLACEHOLDER.sub(value, template)


def complete_png(path: Path) -> bool:
    """Снимок дописан до конца: у PNG последний блок — `IEND`."""
    if not path.is_file():
        return False
    return path.read_bytes().endswith(b"IEND\xaeB`\x82")


def shoot(chrome: str, page: Path, out: Path, timeout: float = 90.0) -> None:
    """Снять страницу браузером в PNG ровно 1200×630.

    Профиль — временный: рисовать карточки в профиле пользователя значит
    зависеть от его расширений, зума и темы, а на открытом Chrome — упереться
    в замок профиля.

    Со своим `--user-data-dir` Chrome (проверено на 151.0.7922.109, macOS)
    записывает снимок и не выходит: процесс живёт до убийства. Поэтому ждём не
    кода возврата, а готового файла — PNG считается дописанным по блоку `IEND`
    — и снимаем процесс сами. Без своего профиля тот же запуск выходит сам, но
    цену за это платит профиль пользователя, и это дороже.
    """
    if out.is_file():
        out.unlink()
    with tempfile.TemporaryDirectory(prefix="unica-og-profile-") as profile:
        process = subprocess.Popen(
            [
                chrome,
                "--headless=new",
                "--disable-gpu",
                "--hide-scrollbars",
                "--no-first-run",
                "--no-default-browser-check",
                f"--user-data-dir={profile}",
                "--force-device-scale-factor=1",
                f"--window-size={WIDTH},{HEIGHT}",
                f"--screenshot={out}",
                page.as_uri(),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        try:
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if complete_png(out) or process.poll() is not None:
                    break
                time.sleep(0.2)
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
            stderr = process.communicate()[1]
    if not complete_png(out):
        raise SystemExit(f"{out.name}: браузер не снял карточку\n{(stderr or '').strip()}")


def render(chrome: str, template: str, page_path: Path, out: Path, work: Path) -> None:
    source = work / f"{page_path.stem}.html"
    source.write_text(render_html(template, card_fields(page_path)), encoding="utf-8")
    shoot(chrome, source, out)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--check",
        action="store_true",
        help="не переписывать карточки, а сверить их с тем, что рендерится сейчас",
    )
    args = parser.parse_args()

    chrome = find_chrome()
    template = TEMPLATE.read_text(encoding="utf-8")
    pages = sorted(PAGES.glob("*.html"))
    if not pages:
        raise SystemExit(f"{PAGES}: нет страниц")

    stale = []
    with tempfile.TemporaryDirectory(prefix="unica-og-") as tmp:
        work = Path(tmp)
        for page_path in pages:
            target = CARDS / f"og-{page_path.stem}.png"
            if args.check:
                fresh = work / target.name
                render(chrome, template, page_path, fresh, work)
                if not target.is_file() or target.read_bytes() != fresh.read_bytes():
                    stale.append(target.name)
                continue
            render(chrome, template, page_path, target, work)
            print(f"{target.relative_to(REPO_ROOT)}: {target.stat().st_size} байт")

    if stale:
        raise SystemExit(
            "карточки разошлись со страницами: "
            + ", ".join(stale)
            + "\nперерисуйте: python scripts/dev/render-social-cards.py"
        )
    if args.check:
        print(f"карточек сверено: {len(pages)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
