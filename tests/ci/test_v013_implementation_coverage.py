"""The v0.13 implementation ledger must state executable support truth."""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
COVERAGE = REPO_ROOT / "arch/tool-implementation-coverage.json"
CATALOG = REPO_ROOT / "crates/unica-coder/src/application/v13/tool_catalog.rs"

SUBJECT_TOOLS = {
    "unica.view",
    "unica.apply",
    "unica.find",
    "unica.search",
    "unica.check",
    "unica.diff",
    "unica.run",
    "unica.docs",
}
COMPATIBILITY_TOOLS = {
    "unica.task.get",
    "unica.task.result",
    "unica.task.cancel",
}
STATUSES = {"supported", "partial", "unsupported", "removed"}
RUN_NAME_ARM = re.compile(r'RunIntent::([A-Za-z0-9_]+)\s*=>\s*"([^"]+)"')
RUN_VARIANT = re.compile(r"RunIntent::([A-Za-z0-9_]+)")


def catalog_run_operations() -> set[str]:
    source = CATALOG.read_text(encoding="utf-8")
    name_method = source.split("pub(crate) const fn name", 1)[1].split(
        "pub(crate) const fn description", 1
    )[0]
    names_by_variant = dict(RUN_NAME_ARM.findall(name_method))
    dictionary = source.split("fn run_dictionary()", 1)[1].split(
        "fn result_envelope_schema()", 1
    )[0]
    variants = set(RUN_VARIANT.findall(dictionary))
    missing_names = variants - names_by_variant.keys()
    if missing_names:
        raise AssertionError(
            f"run dictionary variants have no public names: {sorted(missing_names)}"
        )
    return {names_by_variant[variant] for variant in variants}


def assert_test_evidence(test: unittest.TestCase, evidence: object, location: str) -> None:
    test.assertIsInstance(evidence, list, f"{location}.evidence must be an array")
    for index, item in enumerate(evidence):
        item_location = f"{location}.evidence[{index}]"
        test.assertIsInstance(item, dict, f"{item_location} must be an object")
        test.assertEqual(
            set(item), {"file", "test"}, f"{item_location} shape must stay closed"
        )
        relative = item["file"]
        selector = item["test"]
        test.assertIsInstance(relative, str, f"{item_location}.file must be a string")
        test.assertIsInstance(selector, str, f"{item_location}.test must be a string")
        test.assertTrue(relative and selector, f"{item_location} must be non-empty")
        path = REPO_ROOT / relative
        test.assertTrue(path.is_file(), f"{item_location}.file does not exist: {relative}")
        source = path.read_text(encoding="utf-8")
        test.assertRegex(
            source,
            rf"#\[(?:[A-Za-z0-9_:]+::)?test(?:\([^\]]*\))?\]\s*"
            rf"(?:async\s+)?fn\s+{re.escape(selector)}\s*\(",
            f"{item_location}.test is not an executable test in {relative}",
        )


class V013ImplementationCoverageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.coverage = json.loads(COVERAGE.read_text(encoding="utf-8"))

    def test_record_has_the_closed_top_level_shape(self) -> None:
        self.assertEqual(
            set(self.coverage),
            {"schemaVersion", "surfaceRelease", "subjectTools", "runOperations", "compatibilityTools"},
        )
        self.assertEqual(self.coverage["schemaVersion"], 1)
        self.assertEqual(self.coverage["surfaceRelease"], "v0.13")

    def test_record_covers_exactly_the_public_tools_and_run_dictionary(self) -> None:
        self.assertEqual(set(self.coverage["subjectTools"]), SUBJECT_TOOLS)
        self.assertEqual(set(self.coverage["compatibilityTools"]), COMPATIBILITY_TOOLS)
        self.assertEqual(set(self.coverage["runOperations"]), catalog_run_operations())
        self.assertEqual(len(self.coverage["runOperations"]), 12)
        self.assertNotIn("query.execute", self.coverage["runOperations"])

    def test_every_entry_uses_a_closed_status_and_honest_evidence(self) -> None:
        for group_name in ("subjectTools", "runOperations", "compatibilityTools"):
            for name, entry in sorted(self.coverage[group_name].items()):
                location = f"{group_name}.{name}"
                with self.subTest(location=location):
                    self.assertIsInstance(entry, dict)
                    self.assertEqual(
                        set(entry), {"status", "reason", "evidence"},
                        f"{location} shape must stay closed",
                    )
                    status = entry["status"]
                    self.assertIn(status, STATUSES)
                    reason = entry["reason"]
                    self.assertIsInstance(reason, str)
                    if status in {"partial", "unsupported"}:
                        self.assertTrue(
                            reason.strip(), f"{location} must explain incomplete support"
                        )
                    assert_test_evidence(self, entry["evidence"], location)
                    if status == "supported":
                        self.assertTrue(
                            entry["evidence"],
                            f"{location} supported status requires executable evidence",
                        )

    def test_runtime_truth_supports_only_workspace_initialization(self) -> None:
        for name, entry in self.coverage["runOperations"].items():
            with self.subTest(operation=name):
                expected = (
                    "supported"
                    if name == "workspace.initialize"
                    else "unsupported"
                )
                self.assertEqual(entry["status"], expected)
        self.assertEqual(
            self.coverage["runOperations"]["workspace.initialize"]["status"],
            "supported",
        )
        self.assertNotIn("syntax.check", self.coverage["runOperations"])
        self.assertNotIn("test.run", self.coverage["runOperations"])
        self.assertNotIn("query.execute", self.coverage["runOperations"])

    def test_initial_truth_marks_useful_subject_modes_partial_and_task_transport_supported(self) -> None:
        self.assertEqual(
            {entry["status"] for entry in self.coverage["subjectTools"].values()},
            {"partial"},
        )
        self.assertEqual(
            {entry["status"] for entry in self.coverage["compatibilityTools"].values()},
            {"supported"},
        )


if __name__ == "__main__":
    unittest.main()
