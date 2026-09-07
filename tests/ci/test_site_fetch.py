"""Чтение с сайта: 404 — «файла нет», всё остальное — остановка сборки."""

from __future__ import annotations

import importlib.util
import io
import tempfile
import unittest
import urllib.error
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "site_fetch.py"


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


class SiteFetchTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()
        self.target = Path(tempfile.mkdtemp(prefix="fetch-")) / "nested" / "file.bin"

    def fetch_with(self, opener):
        self.module.urllib.request.urlopen = opener
        return self.module.fetch("https://example.invalid/data/main/run.json", self.target)

    def test_found_file_is_written(self) -> None:
        self.assertTrue(self.fetch_with(lambda url, timeout: Response(b"payload")))
        self.assertEqual(self.target.read_bytes(), b"payload")

    def test_404_means_absent_and_writes_nothing(self) -> None:
        def missing(url, timeout):
            raise urllib.error.HTTPError(url, 404, "Not Found", {}, io.BytesIO())

        self.assertFalse(self.fetch_with(missing))
        self.assertFalse(self.target.exists())

    def test_any_other_answer_stops_the_build(self) -> None:
        """Сбой сети, прочитанный как «файла нет», стёр бы память сайта молча."""
        def unavailable(url, timeout):
            raise urllib.error.HTTPError(url, 503, "Service Unavailable", {}, io.BytesIO())

        def unreachable(url, timeout):
            raise urllib.error.URLError("connection reset")

        for opener in (unavailable, unreachable):
            with self.subTest(opener=opener.__name__):
                with self.assertRaises(SystemExit):
                    self.fetch_with(opener)
                self.assertFalse(self.target.exists())


if __name__ == "__main__":
    unittest.main()
