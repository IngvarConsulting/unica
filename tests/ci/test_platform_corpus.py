"""Корпус выгрузок платформы покрывает каждую измеренную версию формата.

Версии формата выгрузки, которые профиль формата считает измеренными
(`MEASURED_EXPORT_FORMATS`), должны иметь в корпусе выгрузку пустой
конфигурации и пустого расширения. Корень каждого объекта метаданных в этих
выгрузках называет ту же версию. Скрипт сравнения корпусов отличает файлы
с разными байтами от файлов только на одной стороне.
"""

from __future__ import annotations

import importlib.util
import io
import re
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS_ROOT = REPO_ROOT / "tests" / "fixtures" / "formats" / "platform_xml"
FORMAT_PROFILE = (
    REPO_ROOT / "crates" / "unica-source-designer" / "src" / "format_profile.rs"
)
EXPORTS = (
    Path("empty_configuration") / "src",
    Path("empty_extension") / "ext",
)
ROOT_VERSION = re.compile(rb"<MetaDataObject\b[^>]*?\sversion=\"([^\"]*)\"", re.S)
MEASURED = re.compile(r"const\s+MEASURED_EXPORT_FORMATS\s*:[^=]*=\s*&?\[(.*?)\];", re.S)


def measured_formats() -> list:
    match = MEASURED.search(FORMAT_PROFILE.read_text(encoding="utf-8"))
    if not match:
        raise AssertionError(f"{FORMAT_PROFILE}: нет MEASURED_EXPORT_FORMATS")
    body = re.sub(r"//[^\n]*", "", match.group(1))
    return re.findall(r'"([^"]+)"', body)


def load_diff_script():
    path = REPO_ROOT / "scripts" / "dev" / "platform-corpus-diff.py"
    spec = importlib.util.spec_from_file_location("platform_corpus_diff", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PlatformCorpusTests(unittest.TestCase):
    def test_every_measured_format_has_corpus(self) -> None:
        formats = measured_formats()
        self.assertTrue(formats)
        for fmt in formats:
            for export in EXPORTS:
                root = CORPUS_ROOT / fmt / export
                with self.subTest(format=fmt, export=export.as_posix()):
                    self.assertTrue(
                        (root / "Configuration.xml").is_file(), f"нет выгрузки {root}"
                    )
                    for path in sorted(root.rglob("*.xml")):
                        match = ROOT_VERSION.search(path.read_bytes())
                        if match is None:
                            # Не объект метаданных, например интерфейс клиента.
                            continue
                        self.assertEqual(match.group(1).decode(), fmt, path)


class PlatformCorpusDiffTests(unittest.TestCase):
    def test_reports_changed_and_one_sided_files(self) -> None:
        diff = load_diff_script()
        with tempfile.TemporaryDirectory() as tmp:
            left, right = Path(tmp, "left"), Path(tmp, "right")
            for side, version, extra in ((left, "2.20", None), (right, "2.21", "only.xml")):
                (side / "a").mkdir(parents=True)
                (side / "a" / "Configuration.xml").write_bytes(
                    b'<MetaDataObject version="%s"><X/></MetaDataObject>' % version.encode()
                )
                (side / "same.txt").write_bytes(b"same")
                if extra:
                    (side / extra).write_bytes(b"x")
            out = io.StringIO()
            self.assertEqual(diff.compare(left, right, True, out), 1)
            report = out.getvalue()
            self.assertIn("differs  a/Configuration.xml  version 2.20 -> 2.21", report)
            self.assertIn("same     same.txt", report)
            self.assertIn("only.xml", report)
            # Версия корня замаскирована: других отличий в файле нет.
            self.assertNotIn('-<MetaDataObject version="2.20">', report)

            out = io.StringIO()
            self.assertEqual(diff.compare(left, left, False, out), 0)


if __name__ == "__main__":
    unittest.main()
