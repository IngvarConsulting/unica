"""Относительный путь в сообщении продукта пишется через `/` на любой ОС.

`Path::display` соединяет компоненты разделителем хоста. На Windows агент
получал `Catalogs\\Products.xml`, на других ОС — `Catalogs/Products.xml`, и одно
имя файла нельзя было перенести между хостами (#1969). Сообщения печатают
относительный путь через `unica_source_contract::portable_path::PortablePathExt`.

Страж ловит возврат `relative.display()`, `logical_path.display()` и подобных
(`<имя>_relative`, `logical`) в коде продукта без запуска на Windows; путь,
собранный выражением, он не видит. Тестовые модули и файлы тестов не в счёт:
их сообщения агент не видит.
"""

from __future__ import annotations

import re
import subprocess
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
NATIVE_DISPLAY = re.compile(r"\b(?:\w+_)?(?:relative|logical)(?:_path)?\.display\(\)")
# The repository's test files: `*_tests.rs`, `tests.rs`, test support modules.
TEST_FILE = re.compile(r"(?:^|_)tests\.rs$|test_support\.rs$|^testing\.rs$|^test_control\.rs$")
TEST_MODULE = re.compile(r"^#\[cfg\(test\)\]\s*\n(?:pub(?:\([^)]*\))? )?mod \w+\s*\{", re.MULTILINE)


def product_source(text: str) -> str:
    """The file up to its first inline test module."""
    match = TEST_MODULE.search(text)
    return text if match is None else text[: match.start()]


def native_relative_displays(paths: list[Path]) -> list[str]:
    found = []
    for path in paths:
        relative = path.relative_to(REPO_ROOT).as_posix()
        if "/tests/" in relative or TEST_FILE.search(path.name):
            continue
        source = product_source(path.read_text(encoding="utf-8"))
        for match in NATIVE_DISPLAY.finditer(source):
            line = source.count("\n", 0, match.start()) + 1
            found.append(f"{relative}:{line}: {match.group(0)}")
    return found


class PortablePathTests(unittest.TestCase):
    def test_product_messages_spell_relative_paths_portably(self) -> None:
        tracked = subprocess.run(
            ["git", "ls-files", "crates/*/src/**.rs", "crates/*/src/*.rs"],
            cwd=REPO_ROOT, capture_output=True, text=True, check=True,
        ).stdout.split()
        self.assertTrue(tracked, "no Rust sources were listed")
        self.assertEqual(
            native_relative_displays([REPO_ROOT / path for path in tracked]), [],
            "print a relative path with `.portable()` from unica_source_contract::portable_path",
        )

    def test_the_guard_sees_product_code_and_skips_test_modules(self) -> None:
        product = 'fn refuse(relative: &Path) -> String {\n    format!("{}", relative.display())\n}\n'
        tests = '#[cfg(test)]\nmod tests {\n    fn f(relative_path: &Path) { relative_path.display(); }\n}\n'
        self.assertEqual(NATIVE_DISPLAY.findall(product_source(product + tests)), ["relative.display()"])
        self.assertEqual(NATIVE_DISPLAY.findall(product_source(tests)), [])
        self.assertEqual(NATIVE_DISPLAY.findall("relative.portable()"), [])
        self.assertEqual(NATIVE_DISPLAY.findall("descriptor_relative.display()"), ["descriptor_relative.display()"])
        self.assertEqual(NATIVE_DISPLAY.findall("owned.logical_path.display()"), ["logical_path.display()"])
        for name in ["form_lifecycle_tests.rs", "tests.rs", "test_support.rs", "receipt_ledger_test_support.rs", "testing.rs"]:
            self.assertTrue(TEST_FILE.search(name), name)
        for name in ["latest.rs", "attestation.rs", "contest.rs"]:
            self.assertFalse(TEST_FILE.search(name), name)
        # A declared test file is not a body: the product code after it counts.
        declared = "#[cfg(test)]\nmod tests;\n" + product
        self.assertEqual(NATIVE_DISPLAY.findall(product_source(declared)), ["relative.display()"])


if __name__ == "__main__":
    unittest.main()
