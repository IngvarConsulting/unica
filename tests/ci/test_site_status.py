"""Страница статуса: какие линии на ней стоят и что раздаёт канал кандидатов."""

from __future__ import annotations

import base64
import importlib.util
import json
import subprocess
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

if __name__ == "__main__":
    unittest.main()
