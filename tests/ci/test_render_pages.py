"""Главная показывает тесты по версиям: стабильная, кандидат и последние alpha.

Карточки собираются из документа статуса. Здесь страница рендерится тем, что
выдаёт `site-status.py`: ключи карточки версии обязаны совпасть с блоком
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
    {"tag_name": tag, "published_at": at, "html_url": f"{RELEASES_URL}/tag/{tag}", "body": body}
    for tag, at, body in (
        ("v0.13.0-rc.3", "2026-09-28T20:58:22Z", ""),
        ("v0.12.3", "2026-08-19T14:54:10Z", ""),
        ("v0.13.0-alpha.7", "2026-09-30T03:00:00Z", "main@" + "c" * 40),
        ("v0.13.0-alpha.8", "2026-10-01T03:00:00Z", "main@" + "d" * 40),
        ("v0.13.0-alpha.6", "2026-09-29T03:00:00Z", "main@" + "b" * 40),
    )
]


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class AlphaPageTests(unittest.TestCase):
    """На страницу alpha ведут прямой ссылкой, поэтому сайт о ней молчит."""

    def test_the_page_is_closed_to_robots_and_nothing_links_to_it(self) -> None:
        pages = REPO_ROOT / "docs" / "pages"
        self.assertIn('<meta name="robots" content="noindex, nofollow">', (pages / "alpha.html").read_text(encoding="utf-8"))
        for page in pages.glob("*.html"):
            if page.name != "alpha.html":
                with self.subTest(page=page.name):
                    self.assertNotIn("alpha.html", page.read_text(encoding="utf-8"))


class AlphaListTests(unittest.TestCase):
    """Страница alpha показывает все alpha из Releases, новейшую первой."""

    def test_two_alphas_are_both_shown_newest_first_and_downloads_take_the_newest(self) -> None:
        render = load("render_pages", REPO_ROOT / "scripts" / "ci" / "render-pages.py")
        site_status = load("site_status", REPO_ROOT / "scripts" / "ci" / "site-status.py")
        template = (REPO_ROOT / "docs" / "pages" / "alpha.html").read_text(encoding="utf-8")
        releases = [release for release in RELEASES if release["tag_name"] in ("v0.13.0-alpha.7", "v0.13.0-alpha.8")]
        outside = render.REPEAT.sub("", template)
        status: dict[str, object] = {name: f"<{name}>" for name in render.placeholders(outside)}
        status.update(site_status.latest_alpha(releases, "IngvarConsulting/unica"))
        status["alphas"] = site_status.alpha_list(releases, "IngvarConsulting/unica")
        page = render.render(template, status)
        newest, older = page.find(">v0.13.0-alpha.8<"), page.find(">v0.13.0-alpha.7<")
        self.assertGreater(newest, 0)
        self.assertGreater(older, newest)
        self.assertIn(f'commit/{"d" * 40}"', page)
        self.assertIn(f'commit/{"c" * 40}"', page)
        self.assertIn(f'href="{RELEASES_URL}/tag/v0.13.0-alpha.7"', page)
        self.assertIn(f"{RELEASES_URL}/download/v0.13.0-alpha.8</span>/unica-full-linux-x64.tar.gz", page)
        self.assertNotIn("{{", page)


class HomeStateCardsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.render = load("render_pages", REPO_ROOT / "scripts" / "ci" / "render-pages.py")
        self.site_status = load("site_status", REPO_ROOT / "scripts" / "ci" / "site-status.py")
        self.template = HOME.read_text(encoding="utf-8")

    def status(self, next_tag: str) -> dict[str, object]:
        return {"next_tag": next_tag, "next_url": f"{RELEASES_URL}/tag/{next_tag}",
                "stable_tag": "v0.12.3", "version": "v0.12.3"}

    def page(self, next_tag: str, data: Path | None = None) -> str:
        # Значения вне карточек версий этой проверке безразличны: каждое
        # подставляется своим именем, и чужое имя в карточке сразу видно.
        outside = self.render.REPEAT.sub("", self.template)
        status: dict[str, object] = {name: f"<{name}>" for name in self.render.placeholders(outside)}
        status["versions"] = [self.site_status.version_card(kind, release, data, "IngvarConsulting/unica")
                              for kind, release in self.site_status.site_versions(self.status(next_tag), RELEASES)]
        return self.render.render(self.template, status)

    def test_cards_are_the_stable_version_the_candidate_and_the_alphas_prune_keeps(self) -> None:
        kinds = [(kind, release["tag_name"]) for kind, release in self.site_status.site_versions(self.status("v0.13.0-rc.3"), RELEASES)]
        self.assertEqual(kinds, [("Стабильная", "v0.12.3"), ("Кандидат", "v0.13.0-rc.3"),
                                 ("Alpha", "v0.13.0-alpha.8"), ("Alpha", "v0.13.0-alpha.7")])
        self.assertEqual(self.site_status.alphas_kept(), 2)
        # The candidate leaves once the stable version catches up with it.
        kinds = [kind for kind, _ in self.site_status.site_versions(self.status("v0.12.3"), RELEASES)]
        self.assertNotIn("Кандидат", kinds)

    def test_every_card_says_how_to_install_its_kind_of_version_beside_its_report(self) -> None:
        cards = self.site_status.site_versions(self.status("v0.13.0-rc.3"), RELEASES)
        links = {release["tag_name"]: self.site_status.version_card(kind, release, None, "IngvarConsulting/unica")["install_url"]
                 for kind, release in cards}
        self.assertEqual(links, {"v0.12.3": "#install", "v0.13.0-rc.3": "next.html",
                                 "v0.13.0-alpha.8": "alpha.html", "v0.13.0-alpha.7": "alpha.html"})
        page = self.page("v0.13.0-rc.3")
        # The candidate is installed from the `next` channel, not from the stable section.
        self.assertIn('<a href="next.html">Как поставить <span class="vh">v0.13.0-rc.3</span>', page)
        self.assertIn('<a href="#install">Как поставить <span class="vh">v0.12.3</span>', page)
        self.assertIn('<a href="alpha.html">Как поставить <span class="vh">v0.13.0-alpha.8</span>', page)
        # Each footer keeps its report link next to the install link.
        self.assertEqual(page.count("Как поставить <span"), len(cards))
        self.assertEqual(page.count('class="card-foot"'), len(cards))

    def test_a_version_with_a_stored_report_shows_its_counts_skips_by_os_and_report(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp) / "versions" / "v0.13.0-alpha.8"
            folder.mkdir(parents=True)
            (folder / "summary.json").write_text(json.dumps({"statistic": {"total": 785, "passed": 781, "failed": 2, "skipped": 2}}), encoding="utf-8")
            (folder / "breakdown.json").write_text(json.dumps({"by_host": {
                "macos-15": {"total": 300, "passed": 299, "failed": 0, "skipped": 1},
                "ubuntu-latest": {"total": 485, "passed": 482, "failed": 2, "skipped": 1}}}), encoding="utf-8")
            (folder / "run.json").write_text(json.dumps({"sha": "e" * 40}), encoding="utf-8")
            page = self.page("v0.13.0-rc.3", Path(tmp))
        self.assertIn("785 тестов", page)
        self.assertIn("781 прошёл", page)
        self.assertIn("2 упали", page)
        self.assertIn("пропуски по ОС: Linux 1 · macOS 1", page)
        self.assertIn('href="allure/versions/v0.13.0-alpha.8/"', page)
        self.assertIn(f'commit/{"e" * 40}"', page)
        self.assertNotIn("{{", page)

    def test_a_version_without_a_report_says_so_and_uses_its_run_log_counts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp) / "versions" / "v0.12.3"
            folder.mkdir(parents=True)
            (folder / "legacy.json").write_text(json.dumps({"sha": "f" * 40, "url": "https://example.invalid/run/1", "by_host": {
                "ubuntu-latest": {"total": 10, "passed": 9, "failed": 0, "skipped": 1}}}), encoding="utf-8")
            page = self.page("v0.13.0-rc.3", Path(tmp))
        self.assertIn("Отчёта нет — счётчики из журнала прогона", page)
        self.assertIn('href="https://example.invalid/run/1"', page)
        self.assertIn("10 тестов", page)
        # An alpha with neither a report nor a log still names its slice and says there is no report.
        self.assertIn(f'commit/{"d" * 40}"', page)
        self.assertIn(">Отчёта нет <span", page)
        self.assertNotIn("{{", page)


if __name__ == "__main__":
    unittest.main()
