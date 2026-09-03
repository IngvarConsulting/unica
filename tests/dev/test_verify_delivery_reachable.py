"""Адрес поставки проверяет только этот шаг.

Байты движка сверены на сборке, ядро — `verify-release-assets.py`. Адрес в
опубликованном манифесте после сборки не трогает никто, и опечатка в теге
дожила бы до первого вызова движка у пользователя.
"""
from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch

REPO_ROOT = Path(__file__).resolve().parents[2]


def load_module():
    path = REPO_ROOT / "scripts" / "ci" / "verify-delivery-reachable.py"
    spec = importlib.util.spec_from_file_location("delivery_reachable", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CORE_URL = (
    "https://github.com/IngvarConsulting/unica/releases/download/"
    "v0.13.0/unica-runtime-linux-x64.tar.gz"
)
ENGINE_URL = (
    "https://github.com/IngvarConsulting/unica-toolchain/releases/download/"
    "bsl-analyzer-v0.2.67-build.1/bsl-analyzer-linux-x64"
)


def manifest(**overrides) -> dict:
    document = {
        "schemaVersion": 2,
        "pluginVersion": "0.13.0",
        "development": False,
        "artifacts": {
            "unica": {
                "version": "0.13.0",
                "role": "core",
                "targets": {"linux-x64": {"asset": {"url": CORE_URL}}},
            },
            "bsl-analyzer": {
                "version": "0.2.67",
                "role": "engine",
                "targets": {"linux-x64": {"asset": {"url": ENGINE_URL}}},
            },
        },
    }
    document.update(overrides)
    return document


class DeliveryReachableTests(unittest.TestCase):
    def write(self, document: dict) -> Path:
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        path = root / "runtime-manifest.json"
        path.write_text(json.dumps(document), encoding="utf-8")
        return path

    def test_every_declared_address_is_checked_not_only_the_core(self) -> None:
        module = load_module()
        path = self.write(manifest())
        asked: list[str] = []

        def head(url: str, *, timeout: float) -> int:
            asked.append(url)
            return 200

        with patch.object(module, "reachable", side_effect=head):
            self.assertEqual(module.verify(path, timeout=1), 2)

        self.assertEqual(sorted(asked), sorted([CORE_URL, ENGINE_URL]))

    def test_an_address_that_does_not_resolve_fails_the_release(self) -> None:
        module = load_module()
        path = self.write(manifest())

        def head(url: str, *, timeout: float) -> int:
            if url == ENGINE_URL:
                raise urllib.error.HTTPError(url, 404, "Not Found", {}, None)
            return 200

        with patch.object(module, "reachable", side_effect=head):
            with self.assertRaisesRegex(SystemExit, "404"):
                module.verify(path, timeout=1)

    def test_a_broken_channel_is_told_apart_from_a_missing_asset(self) -> None:
        module = load_module()
        path = self.write(manifest())

        with patch.object(module, "reachable", side_effect=TimeoutError()):
            with self.assertRaisesRegex(SystemExit, "TimeoutError"):
                module.verify(path, timeout=1)

    def test_a_development_manifest_is_refused_rather_than_passed(self) -> None:
        # У него ассетов нет вовсе, и молчаливый успех сказал бы, что всё
        # доступно, — ровно то, чего проверка не должна говорить никогда.
        module = load_module()
        path = self.write(manifest(development=True, artifacts={}))

        with self.assertRaisesRegex(SystemExit, "development manifest"):
            module.verify(path, timeout=1)

    def test_a_manifest_of_another_schema_is_refused(self) -> None:
        module = load_module()
        path = self.write(manifest(schemaVersion=3))

        with self.assertRaisesRegex(SystemExit, "schemaVersion"):
            module.verify(path, timeout=1)


if __name__ == "__main__":
    unittest.main()
