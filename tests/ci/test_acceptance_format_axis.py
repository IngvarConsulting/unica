"""The format axis of the acceptance corpus (#2730).

A writing scenario on a format-axis fixture runs once per export format,
over the reference export the platform wrote in that format, and its file
assertions may compare the result with that same export."""
from __future__ import annotations

import json
import re
import tempfile
import unittest
from pathlib import Path

from tests.ci.acceptance_controls import content_mismatches, validate_controls
from tests.ci.acceptance_profiles import (
    ACCEPTANCE_ROOT, REPO, SOURCE_PROFILE_DIR, SOURCE_WORKSPACE_PROFILES, expand_format_axis,
    load_source_workspace_profiles, materialize_format, platform_corpus,
)
from tests.ci.test_acceptance_scenarios import corpus_segments

AXIS = "tests/fixtures/acceptance/workspace-all-kinds"
MD = "{http://v8.1c.ru/8.3/MDClasses}"
VALUE = f"./{MD}Enum/{MD}ChildObjects/{MD}EnumValue[last()]/{MD}Properties/*"


def writable_formats() -> list[str]:
    """Export formats the codecs write, as the source contract declares them.

    `ACTIVE_SERIALIZATION_FORMAT` is the 2.20 row, and every later row is an
    arm of `PlatformXmlRelease::export_format`."""
    crate = REPO / "crates/unica-source-contract/src"
    active = re.search(r'pub const ACTIVE_SERIALIZATION_FORMAT: &str = "(2\.[0-9]+)";',
                       (crate / "lib.rs").read_text(encoding="utf-8"))
    body = re.search(r"pub const fn export_format\(self\) -> &'static str \{(.*?)\n    \}",
                     (crate / "format_identity.rs").read_text(encoding="utf-8"), re.DOTALL)
    if active is None or body is None:
        raise AssertionError("the source contract no longer declares its export formats where this guard reads them")
    releases = re.findall(r'Self::\w+ => "(2\.[0-9]+)"', body.group(1))
    if not releases:
        raise AssertionError("PlatformXmlRelease::export_format names no format")
    return [active.group(1), *releases]


def enum_file(*properties: str) -> str:
    values = "".join(f"<{name}>x</{name}>" for name in properties)
    return (f'<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Enum><ChildObjects>'
            f'<EnumValue><Properties>{values}</Properties></EnumValue>'
            f'</ChildObjects></Enum></MetaDataObject>')


class FormatAxisGuardTests(unittest.TestCase):
    def test_every_writable_format_has_a_reference_export_and_runs_the_axis(self) -> None:
        formats = writable_formats()
        for export_format in formats:
            with self.subTest(format=export_format):
                self.assertTrue((platform_corpus() / export_format / "all_kinds/src/Configuration.xml").is_file(),
                                f"format {export_format} has no all_kinds reference export")
        self.assertEqual(SOURCE_WORKSPACE_PROFILES[AXIS]["formats"], formats,
                         "the writing scenarios of the format axis must run in every writable format")


class FormatAxisDeclarationTests(unittest.TestCase):
    def fixture(self, root: Path, formats, *, with_src=False, references=("2.20", "2.21")) -> Path:
        acceptance = root / "tests/fixtures/acceptance"
        (acceptance / "workspace-axis").mkdir(parents=True)
        if with_src:
            (acceptance / "workspace-axis/src").mkdir()
        for export_format in references:
            (root / f"tests/fixtures/formats/platform_xml/{export_format}/all_kinds/src").mkdir(parents=True)
        (acceptance / SOURCE_PROFILE_DIR).mkdir()
        (acceptance / SOURCE_PROFILE_DIR / "workspace-axis.json").write_text(
            json.dumps({"profile": "source", "formats": formats}), encoding="utf-8")
        return acceptance

    def test_declared_formats_with_references_are_admitted(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            acceptance = self.fixture(Path(raw), ["2.20", "2.21"])
            self.assertEqual(load_source_workspace_profiles(acceptance)[
                "tests/fixtures/acceptance/workspace-axis"]["formats"], ["2.20", "2.21"])

    def test_an_invalid_axis_fails_closed(self) -> None:
        cases = {
            "not a list": ("2.20", {}), "empty": ([], {}), "duplicate": (["2.20", "2.20"], {}),
            "not a format": (["8.5.1"], {}), "number": ([2.2], {}),
            "no reference": (["2.22"], {}), "own src": (["2.20"], {"with_src": True}),
        }
        for case, (formats, options) in cases.items():
            with self.subTest(case=case), tempfile.TemporaryDirectory() as raw:
                acceptance = self.fixture(Path(raw), formats, **options)
                with self.assertRaises(ValueError):
                    load_source_workspace_profiles(acceptance)

    def test_a_run_takes_src_from_its_format(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            acceptance = self.fixture(Path(raw), ["2.20", "2.21"])
            for export_format in ("2.20", "2.21"):
                source = Path(raw) / f"tests/fixtures/formats/platform_xml/{export_format}/all_kinds/src"
                (source / "Configuration.xml").write_text(export_format, encoding="utf-8")
            workspace = Path(raw) / "copy"
            workspace.mkdir()
            materialize_format(workspace, "2.21", acceptance)
            self.assertEqual((workspace / "src/Configuration.xml").read_text(encoding="utf-8"), "2.21")
            plain = Path(raw) / "plain"
            plain.mkdir()
            materialize_format(plain, None, acceptance)
            self.assertFalse((plain / "src").exists())


class FormatAxisExpansionTests(unittest.TestCase):
    PROFILES = {"axis": {"formats": ["2.20", "2.21", "2.22"]}, "plain": {}}

    def test_runs_stay_together_format_by_format_where_the_fixture_first_stood(self) -> None:
        scenarios = [{"id": "A", "workspace": "plain"}, {"id": "B", "workspace": "axis"},
                     {"id": "C", "workspace": "plain"},
                     {"id": "D", "workspace": "axis", "formats": ["2.21"]}]
        expanded = expand_format_axis(scenarios, "plain", self.PROFILES)
        self.assertEqual([(item["id"], item.get("format")) for item in expanded], [
            ("A", None), ("B@2.20", "2.20"), ("B@2.21", "2.21"), ("D@2.21", "2.21"),
            ("B@2.22", "2.22"), ("C", None),
        ])
        self.assertTrue(all("formats" not in item for item in expanded))

    def test_a_narrowing_outside_the_axis_or_without_one_is_rejected(self) -> None:
        for scenario in ({"id": "X", "workspace": "axis", "formats": ["2.23"]},
                         {"id": "X", "workspace": "axis", "formats": []},
                         {"id": "X", "workspace": "axis", "formats": "2.21"},
                         {"id": "X", "workspace": "axis", "formats": ["2.21", "2.21"]},
                         {"id": "X", "workspace": "plain", "formats": ["2.20"]}):
            with self.subTest(scenario=scenario), self.assertRaises(ValueError):
                expand_format_axis([scenario], "plain", self.PROFILES)

    def test_a_server_never_serves_two_formats(self) -> None:
        corpus = {"workspace": "plain", "scenarios": expand_format_axis(
            [{"id": "B", "workspace": "axis", "wire": []},
             {"id": "D", "workspace": "axis", "wire": []}], "plain", self.PROFILES)}
        self.assertEqual([[scenario["id"] for _, scenario in segment] for segment in corpus_segments(corpus)],
                         [["B@2.20", "D@2.20"], ["B@2.21", "D@2.21"], ["B@2.22", "D@2.22"]])


class ReferenceOperandTests(unittest.TestCase):
    STEP = {"tool": "unica.apply", "args": {}, "expect": ["ok"], "fileAssertions": [{
        "path": "src/Enum.xml", "kind": "xmlTags", "select": VALUE,
        "eq": {"$reference": {"path": "src/Enum.xml", "select": VALUE}}}]}

    def run_step(self, written, reference, step=None):
        with tempfile.TemporaryDirectory() as raw:
            workspace, export = Path(raw) / "workspace", Path(raw) / "reference"
            for root, text in ((workspace, written), (export, reference)):
                (root / "src").mkdir(parents=True)
                (root / "src/Enum.xml").write_text(text, encoding="utf-8")
            context = {"reference": str(export)}
            return content_mismatches(step or self.STEP, {"ok": True}, "ok", "", workspace, context)

    def test_the_result_matches_the_export_of_its_own_format(self) -> None:
        validate_controls(self.STEP)
        self.assertEqual(self.run_step(enum_file("Name", "Color"), enum_file("Name", "Color")), [])

    def test_a_result_in_another_format_shape_fails(self) -> None:
        failures = self.run_step(enum_file("Name"), enum_file("Name", "Color"))
        self.assertEqual(len(failures), 1)
        self.assertIn("Color", failures[0])

    def test_a_reference_that_selects_nothing_fails_instead_of_passing_vacuously(self) -> None:
        failures = self.run_step(enum_file(), enum_file())
        self.assertEqual(len(failures), 1)
        self.assertIn("selects nothing", failures[0])

    def test_a_reference_file_missing_from_the_export_fails_instead_of_passing(self) -> None:
        step = {**self.STEP, "fileAssertions": [{**self.STEP["fileAssertions"][0], "eq": {
            "$reference": {"path": "src/Absent.xml", "select": VALUE}}}]}
        failures = self.run_step(enum_file("Name"), enum_file("Name"), step)
        self.assertEqual(len(failures), 1)
        self.assertIn("Absent.xml", failures[0])

    def test_a_reference_needs_a_run_in_a_format(self) -> None:
        failures = content_mismatches(self.STEP, {"ok": True}, "ok", "", Path("."), {})
        self.assertEqual(len(failures), 1)
        self.assertIn("declared format", failures[0])

    def test_an_invalid_reference_is_rejected_by_the_step_validation(self) -> None:
        base = self.STEP["fileAssertions"][0]
        for assertion in ({**base, "eq": {"$reference": {"path": "src/Enum.xml"}}},
                          {**base, "eq": {"$reference": {"path": "../x", "select": VALUE}}},
                          {**base, "eq": {"$reference": {"path": "src/Enum.xml", "select": ""}}},
                          {"path": "src/Enum.xml", "kind": "sha256",
                           "eq": {"$reference": {"path": "src/Enum.xml", "select": VALUE}}}):
            with self.subTest(assertion=assertion), self.assertRaises(ValueError):
                validate_controls({**self.STEP, "fileAssertions": [assertion]})


if __name__ == "__main__":
    unittest.main()
