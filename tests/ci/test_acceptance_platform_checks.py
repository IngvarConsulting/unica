"""Declarations of platform checks name real scenarios and BSL modules.

The platform itself runs only in `tests/research/platform`; here the
declarations are held to their contract so a typo does not wait for a
manual run to surface.
"""
from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from tests.ci.acceptance_platform import load_platform_checks, negative_scenario, read_verdict

CORPUS = {
    "workspace": "ws",
    "scenarios": [{"id": "S-1", "workspace": "ws", "wire": [{"tool": "unica.view"}]}],
}


class PlatformCheckDeclarationTests(unittest.TestCase):
    def test_repository_declarations_load(self) -> None:
        checks = load_platform_checks()
        self.assertIn("S-1703-xdto-property-rename", {check["scenario"] for check in checks})

    def declare(self, root: Path, declaration: dict, name: str = "S-1") -> None:
        directory = root / "tests/fixtures/acceptance/platform-checks"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / f"{name}.json").write_text(json.dumps(declaration), encoding="utf-8")

    def test_a_declaration_is_refused_for_each_broken_contract(self) -> None:
        good = {"scenario": "S-1", "module": "check.bsl", "client": "thick", "pass": "PASS: ok", "negativeControl": "old name"}
        cases = {
            "unknown key": {**good, "extra": 1},
            "missing pass": {key: value for key, value in good.items() if key != "pass"},
            "unknown scenario": {**good, "scenario": "S-2"},
            "missing module": {**good, "module": "absent.bsl"},
            "module in src": {**good, "module": "src/check.bsl"},
            "multiline pass": {**good, "pass": "PASS\nmore"},
            "unknown client": {**good, "client": "web"},
            "failing pass": {**good, "pass": "FAIL: always"},
        }
        for label, declaration in cases.items():
            with self.subTest(label), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                (root / "ws/src").mkdir(parents=True)
                (root / "ws/check.bsl").write_text("", encoding="utf-8")
                (root / "ws/src/check.bsl").write_text("", encoding="utf-8")
                self.declare(root, declaration, declaration.get("scenario", "S-1"))
                with self.assertRaises(ValueError):
                    load_platform_checks(root, CORPUS)
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "ws").mkdir()
            (root / "ws/check.bsl").write_text("", encoding="utf-8")
            self.declare(root, good)
            [check] = load_platform_checks(root, CORPUS)
            self.assertEqual(negative_scenario(check)["wire"], [])
            self.assertEqual(check["workspace"], "ws")

    def test_the_verdict_is_read_through_any_platform_text_encoding(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            path = Path(raw) / "result.txt"
            for payload in (b"\xef\xbb\xbfPASS: x\r\n", "PASS: x".encode("utf-16"), b"PASS: x"):
                path.write_bytes(payload)
                self.assertEqual(read_verdict(path), "PASS: x")


if __name__ == "__main__":
    unittest.main()
