"""Главная показывает стабильную версию всегда, а кандидата — только пока канал впереди.

Карточки собираются из документа статуса. Здесь страница рендерится тем, что
выдаёт `site-status.py`: ключи карточки кандидата обязаны совпасть с блоком
шаблона, иначе сборка сайта откажет, а не выпустит страницу с `{{…}}`.
"""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
HOME = REPO_ROOT / "docs" / "pages" / "index.html"
RELEASES_URL = "https://github.com/IngvarConsulting/unica/releases"
RELEASES = [
    {"tag_name": "v0.13.0-rc.3", "published_at": "2026-09-28T20:58:22Z"},
    {"tag_name": "v0.12.3", "published_at": "2026-08-19T14:54:10Z"},
]


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class HomeStateCardsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.render = load("render_pages", REPO_ROOT / "scripts" / "ci" / "render-pages.py")
        self.site_status = load("site_status", REPO_ROOT / "scripts" / "ci" / "site-status.py")
        self.template = HOME.read_text(encoding="utf-8")

    def page(self, next_tag: str) -> str:
        channel = {
            "next_tag": next_tag,
            "next_url": f"{RELEASES_URL}/tag/{next_tag}",
            "stable_tag": "v0.12.3",
            "version": "v0.12.3",
        }
        # Значения вне карточек версий этой проверке безразличны: каждое
        # подставляется своим именем, и чужое имя в карточке сразу видно.
        outside = self.render.REPEAT.sub("", self.template)
        status: dict[str, object] = {name: f"<{name}>" for name in self.render.placeholders(outside)}
        status.update(version="v0.12.3", version_url=f"{RELEASES_URL}/tag/v0.12.3", version_date="19.08.2026")
        status["candidates"] = self.site_status.candidates(channel, RELEASES)
        with tempfile.TemporaryDirectory() as tmp:
            summary = Path(tmp) / "summary.json"
            summary.write_text(json.dumps({"statistic": {"total": 785, "passed": 781, "failed": 2, "skipped": 2}}), encoding="utf-8")
            counts = self.site_status.summary_counts(summary)
        status["tested_lines"] = [{
            "line": "main", "build_sha": "a548443", "build_date": "29.09.2026",
            "build_url": "https://github.com/IngvarConsulting/unica/actions", "report_url": "allure/main/", **counts,
        }]
        return self.render.render(self.template, status)

    def test_the_candidate_card_leads_to_the_channel_page_while_the_channel_is_ahead(self) -> None:
        page = self.page("v0.13.0-rc.3")

        self.assertIn("Кандидат выпуска", page)
        self.assertIn('<a href="next.html">Как поставить <span class="vh">v0.13.0-rc.3</span> <span aria-hidden="true">→</span></a>', page)
        self.assertIn(f'href="{RELEASES_URL}/tag/v0.13.0-rc.3"', page)
        self.assertIn("опубликован 28.09.2026", page)
        self.assertNotIn("{{", page)

    def test_the_stable_card_stays_and_the_candidate_card_leaves_once_the_release_is_out(self) -> None:
        page = self.page("v0.12.3")

        self.assertNotIn("Кандидат выпуска", page)
        self.assertIn('<span class="metric">v0.12.3</span>', page)
        self.assertIn("опубликована 19.08.2026", page)
        self.assertNotIn("{{", page)

    def test_the_line_card_words_agree_with_their_counts(self) -> None:
        """«785 теста» и «781 прошли» — ошибки; слово приходит со статусом."""
        page = self.page("v0.13.0-rc.3")

        self.assertIn("785 <small>тестов</small>", page)
        self.assertIn("781 прошёл", page)
        self.assertIn("2 упали", page)


if __name__ == "__main__":
    unittest.main()
