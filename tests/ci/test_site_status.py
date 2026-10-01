"""Страница статуса: какие линии на ней стоят и что раздаёт канал кандидатов."""

from __future__ import annotations

import base64
import importlib.util
import json
import subprocess
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "site-status.py"


def load_module():
    spec = importlib.util.spec_from_file_location("site_status", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SiteLinesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()
        self.module.open_lines = lambda repo, now: ["release-v0.12", "release-v0.13"]
        self.now = datetime.now(timezone.utc)

    def lines(self, branch: str) -> list[str]:
        return self.module.site_lines(branch, "IngvarConsulting/unica", self.now)

    def test_main_stays_on_the_page_whatever_line_the_run_came_from(self) -> None:
        self.assertEqual(self.lines("main"), ["main", "release-v0.12", "release-v0.13"])
        self.assertEqual(self.lines("release-v0.13"), ["main", "release-v0.13", "release-v0.12"])

    def test_a_tag_or_a_stray_branch_is_not_a_line_and_gets_no_card(self) -> None:
        """Результаты тега лежат в его линии; карточка «v0.13.2 — нет прогонов» врала бы."""
        for ref in ("v0.13.2", "feature/x", "release-v0.13.1"):
            with self.subTest(ref=ref):
                self.assertEqual(self.lines(ref), ["main", "release-v0.12", "release-v0.13"])


def catalog(ref: str) -> dict:
    """Ответ GitHub на чтение каталога: тело в base64, как отдаёт `contents`."""
    document = {
        "name": "unica-next",
        "plugins": [{"name": "unica", "source": {"source": "git-subdir", "path": "plugins/unica", "ref": ref}}],
    }
    return {"content": base64.b64encode(json.dumps(document).encode("utf-8")).decode("ascii")}


class ChannelTests(unittest.TestCase):
    """Версии каналов называют каталоги веток маркетплейса, а не список релизов."""

    MARKETPLACE = "IngvarConsulting/unica-marketplace"
    RELEASES = "https://github.com/IngvarConsulting/unica/releases"
    CODEX = ".agents/plugins/marketplace.json"
    CLAUDE = ".claude-plugin/marketplace.json"

    def setUp(self) -> None:
        self.module = load_module()
        self.asked: list[str] = []

    def serve(self, branches: dict[str, dict[str, str]]) -> None:
        """Отвечать каталогом только на чтение его пути с его ветки."""

        def gh(repo: str, path: str) -> list:
            self.asked.append(path)
            for branch, refs in branches.items():
                for catalog_path, ref in refs.items():
                    if repo == self.MARKETPLACE and path == f"contents/{catalog_path}?ref={branch}":
                        return [catalog(ref)]
            raise subprocess.CalledProcessError(1, ["gh", "api", path])

        self.module.gh = gh

    def channels(self) -> dict[str, str]:
        return self.module.channels("IngvarConsulting/unica", self.MARKETPLACE)

    def test_each_channel_is_the_release_both_catalogs_of_its_branch_pin(self) -> None:
        self.serve({
            "next": {self.CODEX: "v0.13.0-rc.3", self.CLAUDE: "v0.13.0-rc.3"},
            "main": {self.CODEX: "v0.12.3", self.CLAUDE: "v0.12.3"},
        })

        self.assertEqual(self.channels(), {
            "marketplace": self.MARKETPLACE,
            "releases_url": self.RELEASES,
            "next_tag": "v0.13.0-rc.3",
            "next_url": f"{self.RELEASES}/tag/v0.13.0-rc.3",
            "next_version": "0.13.0-rc.3",
            "stable_tag": "v0.12.3",
            "stable_url": f"{self.RELEASES}/tag/v0.12.3",
        })
        self.assertEqual(len(self.asked), 4)

    def test_catalogs_that_disagree_or_cannot_be_read_name_no_version(self) -> None:
        """Прочерк вместо версии, которую ветка раздаёт не целиком, и сайт собирается дальше."""
        self.serve({
            "next": {self.CODEX: "v0.13.0-rc.3", self.CLAUDE: "v0.13.0-rc.2"},
            "main": {self.CODEX: "v0.12.3"},
        })

        found = self.channels()

        self.assertEqual(
            {key: found[key] for key in ("next_tag", "next_version", "next_url", "stable_tag", "stable_url")},
            {
                "next_tag": "—",
                "next_version": "—",
                "next_url": self.RELEASES,
                "stable_tag": "—",
                "stable_url": self.RELEASES,
            },
        )


class CandidateTests(unittest.TestCase):
    """Карточка кандидата на главной зовёт поставить то, что раздаёт канал."""

    RELEASES = [
        {"tag_name": "v0.13.0-rc.3", "published_at": "2026-09-28T20:58:22Z"},
        {"tag_name": "v0.13.0-rc.2", "published_at": "2026-09-25T22:40:00Z"},
        {"tag_name": "v0.12.3", "published_at": "2026-08-19T14:54:10Z"},
    ]

    def setUp(self) -> None:
        self.module = load_module()

    def candidates(self, next_tag: str, stable_tag: str, version: str) -> list[dict[str, str]]:
        """`version` — последний стабильный релиз; по нему главная показывает стабильную версию."""
        status = {"next_tag": next_tag, "next_url": f"https://releases/tag/{next_tag}", "stable_tag": stable_tag, "version": version}
        return self.module.candidates(status, self.RELEASES)

    def test_a_candidate_ahead_of_the_stable_version_gets_a_card_with_its_own_release_date(self) -> None:
        self.assertEqual(self.candidates("v0.13.0-rc.3", "v0.12.3", "v0.12.3"), [{
            "candidate_tag": "v0.13.0-rc.3",
            "candidate_url": "https://releases/tag/v0.13.0-rc.3",
            "candidate_date": "28.09.2026",
        }])
        # Каталог main не прочитался: кандидат сравнивается с релизом.
        self.assertEqual(len(self.candidates("v0.13.0-rc.3", "—", "v0.12.3")), 1)

    def test_no_card_once_the_release_is_out_or_the_channel_is_unknown(self) -> None:
        for next_tag, stable_tag, version in (("v0.13.0", "v0.13.0", "v0.13.0"), ("—", "v0.12.3", "v0.12.3")):
            with self.subTest(next=next_tag):
                self.assertEqual(self.candidates(next_tag, stable_tag, version), [])

    def test_a_stable_release_is_never_shown_as_a_candidate(self) -> None:
        """Полная версия приходит в next раньше, чем в main; до main она ещё не кандидат."""
        self.assertEqual(self.candidates("v0.13.0", "v0.12.3", "v0.13.0"), [])
        self.assertEqual(self.candidates("v0.13.0", "v0.12.3", "v0.12.3"), [])

    def test_a_candidate_older_than_the_published_release_gets_no_card(self) -> None:
        """Релиз 0.13.0 вышел, а каталоги ещё не сдвинуты: rc.3 уже не впереди."""
        self.assertEqual(self.candidates("v0.13.0-rc.3", "v0.12.3", "v0.13.0"), [])

    def test_release_date_is_a_dash_for_an_unknown_tag(self) -> None:
        self.assertEqual(self.module.release_date(self.RELEASES, "v0.12.3"), "19.08.2026")
        self.assertEqual(self.module.release_date(self.RELEASES, "—"), "—")


class CountWordTests(unittest.TestCase):
    """Слово при числе в карточке линии согласуется с числом."""

    def setUp(self) -> None:
        self.module = load_module()

    def test_the_word_agrees_with_the_count(self) -> None:
        cases = {
            0: "тестов", 1: "тест", 2: "теста", 4: "теста", 5: "тестов", 11: "тестов", 12: "тестов",
            14: "тестов", 21: "тест", 22: "теста", 25: "тестов", 111: "тестов", 785: "тестов", 15653: "теста",
        }
        for count, word in cases.items():
            with self.subTest(count):
                self.assertEqual(self.module.plural(count, "тест", "теста", "тестов"), word)

    def test_the_line_card_counts_carry_their_words(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            summary = Path(tmp) / "summary.json"
            summary.write_text(json.dumps({"statistic": {"total": 785, "passed": 781, "failed": 1, "broken": 1, "skipped": 2}}), encoding="utf-8")
            counts = self.module.summary_counts(summary)

        self.assertEqual(counts, {
            "tests_total": "785", "tests_total_word": "тестов",
            "tests_passed": "781", "tests_passed_word": "прошёл",
            "tests_failed": "2", "tests_failed_word": "упали",
            "tests_skipped": "2", "tests_skipped_word": "пропущены",
        })

if __name__ == "__main__":
    unittest.main()
