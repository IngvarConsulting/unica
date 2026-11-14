"""Корпус выгрузок платформы покрывает каждую измеренную версию формата.

Версии формата выгрузки, которые профиль формата считает измеренными
(`MEASURED_EXPORT_FORMATS`), должны иметь в корпусе выгрузку пустой
конфигурации и пустого расширения. Корень каждого объекта метаданных в этих
выгрузках называет ту же версию. Записываемые форматы несут экспорт XDTO-схем
платформы своей линии: паспорт называет сборку этой линии и каждый файл.
Скрипт сравнения корпусов отличает файлы с разными байтами от файлов только
на одной стороне.
"""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
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
# Версии, для которых снята выгрузка all_kinds: по одному объекту каждого вида.
ALL_KINDS_FORMATS = ("2.20", "2.21", "2.22")
FIRST_ELEMENT_VERSION = re.compile(rb"\A(?:\xef\xbb\xbf)?<\?xml[^>]*\?>\s*<[\w:]+\b[^>]*?\sversion=\"([^\"]*)\"", re.S)
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


# Пустые внешняя обработка и внешний отчёт форматов после 2.20: эталон того,
# что пишет создание внешнего набора в формате своей линии (#2730).
EMPTY_EXTERNAL = {
    "empty_external_processor": Path("epf") / "EmptyProcessor.xml",
    "empty_external_report": Path("erf") / "EmptyReport.xml",
}


class EmptyExternalCorpusTests(unittest.TestCase):
    def test_every_format_after_2_20_has_empty_external_exports_of_its_line(self) -> None:
        for fmt in (fmt for fmt in ALL_KINDS_FORMATS if fmt != "2.20"):
            for folder, relative in EMPTY_EXTERNAL.items():
                root = CORPUS_ROOT / fmt / folder
                with self.subTest(format=fmt, export=folder):
                    data = (root / relative).read_bytes()
                    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
                    self.assertEqual(manifest["format"], fmt)
                    self.assertEqual(manifest["files"], {relative.as_posix(): {
                        "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}})
                    self.assertEqual(ROOT_VERSION.search(data).group(1).decode(), fmt)
                    self.assertTrue(data.startswith(b"\xef\xbb\xbf") and b"\r\n" in data,
                                    "байты выгрузки платформы: BOM и CRLF")

class AllKindsCorpusTests(unittest.TestCase):
    def test_all_kinds_export_names_its_format_everywhere(self) -> None:
        # Корень объекта и подчинённый файл, который несёт версию (форма,
        # права, состав плана обмена, предопределённые), называют версию
        # каталога. Файлы без версии в корне (макеты, WSDL) не проверяются.
        for fmt in ALL_KINDS_FORMATS:
            root = CORPUS_ROOT / fmt / "all_kinds" / "src"
            with self.subTest(format=fmt):
                self.assertTrue((root / "Configuration.xml").is_file(), f"нет выгрузки {root}")
                checked = 0
                for path in sorted(root.rglob("*.xml")):
                    match = FIRST_ELEMENT_VERSION.search(path.read_bytes())
                    if match is None:
                        continue
                    checked += 1
                    self.assertEqual(match.group(1).decode(), fmt, path)
                self.assertGreater(checked, 40, root)


# Записываемые форматы и линии, которые их пишут: экспорт XDTO снят на сборке
# этой линии (scripts/dev/export-platform-xdto.py).
XDTO_FORMAT_LINES = {"2.20": "8.3.27", "2.21": "8.5.1", "2.22": "8.5.4"}


class XdtoCorpusTests(unittest.TestCase):
    def test_every_writable_format_has_the_xdto_export_of_its_line(self) -> None:
        for fmt, line in XDTO_FORMAT_LINES.items():
            root = CORPUS_ROOT / fmt / "xdto"
            with self.subTest(format=fmt):
                manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
                self.assertEqual(manifest["format"], fmt)
                self.assertTrue(manifest["build"].startswith(line + "."), manifest["build"])
                listed = sorted(name for names in manifest["files"].values() for name in names)
                on_disk = sorted(path.name for path in root.glob("*.xsd"))
                self.assertEqual(listed, on_disk)
                self.assertEqual(manifest["packages"], len(manifest["files"]))
                self.assertEqual(manifest["schemas"], len(on_disk))
                digest = hashlib.sha256()
                for name in on_disk:
                    digest.update(name.encode("utf-8") + b"\0" + (root / name).read_bytes())
                self.assertEqual(manifest["sha256"], digest.hexdigest())
                self.assertIn("http://v8.1c.ru/8.3/xcf/enums", manifest["files"])


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
