from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
REFERENCE_SKILLS = ROOT / "tests/fixtures/unica_mcp_script_parity/reference_skills"
SUBSYSTEM_EDIT = REFERENCE_SKILLS / "subsystem-edit/scripts/subsystem-edit.py"
TEMPLATE_ADD = REFERENCE_SKILLS / "template-add/scripts/add-template.py"
META_VALIDATE = REFERENCE_SKILLS / "meta-validate/scripts/meta-validate.py"
MXL_COMPILE = REFERENCE_SKILLS / "mxl-compile/scripts/mxl-compile.py"
DCS_COMPILE = REFERENCE_SKILLS / "dcs-compile/scripts/dcs-compile.py"
VALIDATOR_SCRIPTS = tuple(
    REFERENCE_SKILLS / relative
    for relative in (
        "cf-validate/scripts/cf-validate.py",
        "cfe-validate/scripts/cfe-validate.py",
        "form-validate/scripts/form-validate.py",
        "meta-validate/scripts/meta-validate.py",
        "subsystem-validate/scripts/subsystem-validate.py",
    )
)
MD_NS = "http://v8.1c.ru/8.3/MDClasses"
MXL_NS = "http://v8.1c.ru/8.2/data/spreadsheet"


class ReconfigurableStringIO(io.StringIO):
    def reconfigure(self, **_kwargs) -> None:
        pass


def run_script(script: Path, *arguments: str, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(script), *arguments],
        cwd=cwd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )


def load_script(path: Path, module_name: str):
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot load script: {path}")
    module = importlib.util.module_from_spec(spec)
    previous = sys.dont_write_bytecode
    sys.dont_write_bytecode = True
    try:
        spec.loader.exec_module(module)
    finally:
        sys.dont_write_bytecode = previous
    return module


def subsystem_xml(version: str | None) -> str:
    version_attribute = "" if version is None else f' version="{version}"'
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        f'<MetaDataObject xmlns="{MD_NS}"{version_attribute}>'
        "<Subsystem><Properties><Name>Sales</Name></Properties>"
        "<ChildObjects/><Content/></Subsystem></MetaDataObject>\n"
    )


class ReferenceFormatProfileTests(unittest.TestCase):
    def test_dcs_compile_validates_before_printing_success(self) -> None:
        source = DCS_COMPILE.read_text(encoding="utf-8")

        validation = source.rindex("run_post_validation(output_path)")
        success = source.rindex('print(f"OK  {args.OutputPath}")')

        self.assertLess(validation, success)

    def test_subsystem_edit_rejects_nonexact_owner_before_write(self) -> None:
        for version in (None, "2.19", "2.20.0"):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                subsystem = root / "Sales.xml"
                before = subsystem_xml(version).encode()
                subsystem.write_bytes(before)

                result = run_script(
                    SUBSYSTEM_EDIT,
                    "-SubsystemPath",
                    str(subsystem),
                    "-Operation",
                    "add-content",
                    "-Value",
                    "Catalog.Item",
                    "-NoValidate",
                    cwd=root,
                )

                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("expected exact '2.20'", result.stderr)
                self.assertEqual(subsystem.read_bytes(), before)

    def test_subsystem_edit_restores_parent_if_child_stub_creation_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            subsystem = root / "Sales.xml"
            before = subsystem_xml("2.20").encode()
            subsystem.write_bytes(before)
            conflict = root / "Sales" / "Subsystems"
            conflict.parent.mkdir()
            conflict.write_text("not a directory", encoding="utf-8")

            result = run_script(
                SUBSYSTEM_EDIT,
                "-SubsystemPath",
                str(subsystem),
                "-Operation",
                "add-child",
                "-Value",
                "Broken",
                "-NoValidate",
                cwd=root,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertIn("Failed to publish subsystem edit", result.stderr)
            self.assertEqual(subsystem.read_bytes(), before)

    def test_subsystem_edit_removes_created_directory_chain_on_rollback(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            subsystem = root / "Sales.xml"
            before = subsystem_xml("2.20").encode()
            subsystem.write_bytes(before)
            script = load_script(SUBSYSTEM_EDIT, "reference_subsystem_edit")

            with (
                mock.patch.object(
                    sys,
                    "argv",
                    [
                        str(SUBSYSTEM_EDIT),
                        "-SubsystemPath",
                        str(subsystem),
                        "-Operation",
                        "add-child",
                        "-Value",
                        "Broken",
                        "-NoValidate",
                    ],
                ),
                mock.patch.object(
                    script,
                    "write_child_subsystem_stub",
                    side_effect=OSError("forced child write failure"),
                ),
                contextlib.redirect_stdout(ReconfigurableStringIO()),
                contextlib.redirect_stderr(ReconfigurableStringIO()),
                self.assertRaises(SystemExit) as raised,
            ):
                script.main()

            self.assertEqual(raised.exception.code, 1)
            self.assertEqual(subsystem.read_bytes(), before)
            self.assertFalse((root / "Sales").exists())

    def test_template_add_uses_object_owner_and_rejects_nonexact_version(self) -> None:
        for version in (None, "2.19", "2.20.0"):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                reports = root / "src" / "Reports"
                reports.mkdir(parents=True)
                owner = reports / "Sales.xml"
                owner.write_text(
                    subsystem_xml(version).replace("Subsystem", "Report"),
                    encoding="utf-8",
                )

                result = run_script(
                    TEMPLATE_ADD,
                    "-ObjectName",
                    "Sales",
                    "-TemplateName",
                    "Main",
                    "-TemplateType",
                    "Text",
                    "-SrcDir",
                    "src",
                    cwd=root,
                )

                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("expected exact '2.20'", result.stderr)
                self.assertFalse((reports / "Sales").exists())

    def test_reference_validators_reject_malformed_and_numeric_equivalent_versions(self) -> None:
        for script in VALIDATOR_SCRIPTS:
            with self.subTest(script=script):
                source = script.read_text(encoding="utf-8")
                self.assertIn("re.fullmatch", source)
                self.assertRegex(source, r"actual == ['\"]2\.20['\"]")

        for version in ("-1.0", "+2.20", "2.20.0"):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                owner = root / "Catalog.xml"
                owner.write_text(
                    '<?xml version="1.0" encoding="UTF-8"?>\n'
                    f'<MetaDataObject xmlns="{MD_NS}" version="{version}">'
                    "<Catalog><Properties><Name>Item</Name></Properties></Catalog>"
                    "</MetaDataObject>\n",
                    encoding="utf-8",
                )

                result = run_script(
                    META_VALIDATE,
                    "-ObjectPath",
                    str(owner),
                    cwd=root,
                )

                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn(f"invalid export format version '{version}'", result.stdout)

    def test_reference_mxl_writer_uses_span_for_implicit_next_column(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            definition = root / "mxl.json"
            output = root / "Template.xml"
            definition.write_text(
                json.dumps(
                    {
                        "columns": 5,
                        "areas": [
                            {
                                "name": "A",
                                "rows": [
                                    {
                                        "cells": [
                                            {"col": 1, "span": 2, "text": "spanned"},
                                            {"col": 3, "text": "adjacent"},
                                            {"col": 5, "text": "after gap"},
                                        ]
                                    }
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            result = run_script(
                MXL_COMPILE,
                "-JsonPath",
                str(definition),
                "-OutputPath",
                str(output),
                cwd=root,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            row = ET.parse(output).find(f".//{{{MXL_NS}}}row")
            self.assertIsNotNone(row)
            cells = row.findall(f"{{{MXL_NS}}}c")
            self.assertEqual(len(cells), 3)
            self.assertIsNone(cells[1].find(f"{{{MXL_NS}}}i"))
            self.assertEqual(cells[2].findtext(f"{{{MXL_NS}}}i"), "4")


if __name__ == "__main__":
    unittest.main()
