"""Карточка ссылки доезжает до ленты: страница называет свою, сборка её кладёт.

Картинку `og:image` качает не человек, а робот мессенджера, и молча: битая
ссылка не роняет страницу, не попадает в консоль и видна только тем, кто
поделился ссылкой. Поэтому проводку проверяем здесь — файл существует, лежит
в сайте и имеет ровно тот размер, который страница о нём объявила.

Саму отрисовку страж не воспроизводит: у прогона нет ни браузера, ни системных
шрифтов, которыми набрана карточка. Её перерисовывает
`scripts/dev/render-social-cards.py` на машине с этими шрифтами, а результат
лежит в дереве.
"""

from __future__ import annotations

import importlib.util
import re
import struct
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PAGES = REPO_ROOT / "docs" / "pages"
CARDS = PAGES / "og"
RENDER_PAGES = REPO_ROOT / "scripts" / "ci" / "render-pages.py"

# Пропорция 1.91:1, которую ждут Telegram, X, VK и LinkedIn. Меньшая сторона
# у всех четырёх обрезается по-своему, поэтому размер один и он здесь.
CARD_SIZE = (1200, 630)

META = re.compile(r'<meta property="(?P<key>og:[a-z:]+)" content="(?P<value>[^"]*)">')


def load_render_pages():
    spec = importlib.util.spec_from_file_location("render_pages", RENDER_PAGES)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def og(page: Path) -> dict[str, str]:
    return {m.group("key"): m.group("value") for m in META.finditer(page.read_text(encoding="utf-8"))}


def png_size(path: Path) -> tuple[int, int]:
    """Ширина и высота из блока IHDR — первых двадцати четырёх байт файла."""
    header = path.read_bytes()[:24]
    if header[:8] != b"\x89PNG\r\n\x1a\n":
        raise AssertionError(f"{path.name}: это не PNG")
    return struct.unpack(">II", header[16:24])


def pages() -> list[Path]:
    return sorted(PAGES.glob("*.html"))


class SocialCardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.pages = pages()
        self.assertTrue(self.pages, "в docs/pages нет страниц")
        # Корень сайта берётся у главной страницы, а не пишется здесь второй
        # раз: переезд сайта не должен молчать в двух местах.
        self.site = og(PAGES / "index.html")["og:url"]

    def test_every_page_names_a_card_that_exists(self) -> None:
        """У каждой страницы своя карточка: общая говорила бы за все три сразу."""
        seen = set()
        for page in self.pages:
            with self.subTest(page=page.name):
                image = og(page).get("og:image")
                self.assertIsNotNone(image, "страница не объявила og:image")
                self.assertEqual(image, f"{self.site}assets/og-{page.stem}.png")
                self.assertTrue((CARDS / f"og-{page.stem}.png").is_file(), "карточки нет в дереве")
                self.assertNotIn(image, seen, "две страницы делят одну карточку")
                seen.add(image)

    def test_declared_size_matches_the_file(self) -> None:
        """Объявленный размер — обещание ленте: она верстает превью до загрузки."""
        for page in self.pages:
            with self.subTest(page=page.name):
                meta = og(page)
                declared = (int(meta["og:image:width"]), int(meta["og:image:height"]))
                self.assertEqual(declared, CARD_SIZE)
                self.assertEqual(png_size(CARDS / f"og-{page.stem}.png"), CARD_SIZE)

    def test_every_page_describes_its_card(self) -> None:
        """`og:image:alt` читает тот, кому картинка не видна."""
        for page in self.pages:
            with self.subTest(page=page.name):
                self.assertTrue(og(page).get("og:image:alt", "").strip(), "нет og:image:alt")

    def test_the_site_publishes_every_card(self) -> None:
        """Файл, на который показывает `og:image`, обязан попасть в сайт."""
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            load_render_pages().copy_assets(out)
            published = {path.name for path in (out / "assets").iterdir()}
        for page in self.pages:
            with self.subTest(page=page.name):
                self.assertIn(f"og-{page.stem}.png", published)

    def test_no_card_without_a_page(self) -> None:
        """Карточка страницы, которой больше нет, уезжает на сайт мусором."""
        stems = {f"og-{page.stem}.png" for page in self.pages}
        self.assertEqual({card.name for card in CARDS.glob("*.png")}, stems)


if __name__ == "__main__":
    unittest.main()
