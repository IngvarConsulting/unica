"""Чтение с сайта: «файла нет» отличается от «сайт не отвечает».

404 — файла нет. 503 GitHub Pages отдаёт и на отсутствующие пути (#3099),
поэтому 503 повторяется, а устойчивый 503 считается отсутствием только при
живом сайте. Остальное останавливает сборку: сбой, прочитанный как «файла нет»,
молча стёр бы память сайта при переиздании.
"""

from __future__ import annotations

import importlib.util
import io
import shutil
import tempfile
import unittest
import urllib.error
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "site_fetch.py"
SITE = "https://example.invalid/unica"
FILE = "data/main/run.json"


def load_module():
    spec = importlib.util.spec_from_file_location("site_fetch", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Response:
    def __init__(self, payload: bytes) -> None:
        self.payload = payload

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False

    def read(self) -> bytes:
        return self.payload


def status(code: int):
    def answer(url):
        raise urllib.error.HTTPError(url, code, "status", {}, io.BytesIO())
    return answer


def found(payload: bytes):
    return lambda _url: Response(payload)


class Site:
    """Сайт по сценарию: каждый адрес отвечает своей очередью ответов."""

    def __init__(self, answers: dict[str, list]) -> None:
        self.answers = {f"{SITE}/{path}": list(queue) for path, queue in answers.items()}
        self.requests: list[str] = []

    def __call__(self, url: str, timeout: int):
        self.requests.append(url)
        queue = self.answers[url]
        return (queue.pop(0) if len(queue) > 1 else queue[0])(url)


class SiteFetchTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()
        self.module.PAUSE = 0
        scratch = Path(tempfile.mkdtemp(prefix="fetch-"))
        self.addCleanup(shutil.rmtree, scratch)
        self.target = scratch / "nested" / "file.bin"

    def fetch_from(self, site: Site) -> bool:
        self.module.urllib.request.urlopen = site
        return self.module.fetch(SITE, FILE, self.target)

    def test_found_file_is_written(self) -> None:
        self.assertTrue(self.fetch_from(Site({FILE: [found(b"payload")]})))
        self.assertEqual(self.target.read_bytes(), b"payload")

    def test_404_means_absent_and_writes_nothing(self) -> None:
        site = Site({FILE: [status(404)]})
        self.assertFalse(self.fetch_from(site))
        self.assertFalse(self.target.exists())
        self.assertEqual(site.requests, [f"{SITE}/{FILE}"])

    def test_503_is_retried_until_the_real_answer(self) -> None:
        for answer, expected in ((found(b"payload"), True), (status(404), False)):
            with self.subTest(expected=expected):
                site = Site({FILE: [status(503), answer]})
                self.assertIs(self.fetch_from(site), expected)
                self.assertEqual(site.requests, [f"{SITE}/{FILE}"] * 2)

    def test_steady_503_on_a_live_site_means_absent(self) -> None:
        """Pages отдаёт 503 вместо 404 на отсутствующие пути, а index.html — с 200."""
        site = Site({FILE: [status(503)], "index.html": [found(b"<html>")]})
        self.assertFalse(self.fetch_from(site))
        self.assertFalse(self.target.exists())
        self.assertEqual(
            site.requests,
            [f"{SITE}/{FILE}"] * self.module.ATTEMPTS + [f"{SITE}/index.html"],
        )

    def test_a_site_that_does_not_answer_stops_the_build(self) -> None:
        """Сбой сети, прочитанный как «файла нет», стёр бы память сайта молча."""
        def unreachable(url):
            raise urllib.error.URLError("connection reset")

        cases = {
            "503 и на index.html": {FILE: [status(503)], "index.html": [status(503)]},
            "503, затем обрыв на index.html": {FILE: [status(503)], "index.html": [unreachable]},
            "500": {FILE: [status(500)]},
            "обрыв соединения": {FILE: [unreachable]},
        }
        for name, answers in cases.items():
            with self.subTest(name):
                with self.assertRaises(SystemExit) as stop:
                    self.fetch_from(Site(answers))
                self.assertIn("сайт не отвечает", str(stop.exception))
                self.assertFalse(self.target.exists())


if __name__ == "__main__":
    unittest.main()
