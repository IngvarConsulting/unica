"""The coverage slice must retain missing evidence and reject broken links."""
from __future__ import annotations

from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / "scripts/dev/generate-kind-coverage.py"
SPEC = importlib.util.spec_from_file_location("generate_kind_coverage", SCRIPT)
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)
SCENARIO = "S-Fixture-create"


class KindCoverageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.repo = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.declare("alpha", "Alpha")
        self.declare("beta", "Beta")
        self.wire = [
            {"tool": "unica.apply", "args": {"at": "main:Configuration", "ops": [
                {"op": "object.create", "args": {"values": {"kind": "Alpha", "name": "Probe"}}},
            ]}, "expect": ["ok"], "captures": {"token": {"pointer": "/data/executionToken"}}},
            {"tool": "unica.apply", "args": {"executionToken": "$capture.token"}, "expect": ["ok"]},
            {"tool": "unica.view", "args": {"at": "main:Alpha.Probe"}, "expect": ["ok"],
             "assertions": [{"pointer": "/data/kind", "eq": "Alpha"}]},
        ]
        self.save_scenario()

    def write(self, path: Path | str, text: str) -> None:
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")

    def declare(self, filename: str, kind: str) -> None:
        self.write(f"crates/unica-address/src/kinds/{filename}.rs",
                   f'root: Some(RootPlace {{}}), ..KindDeclaration::new("{kind}")')

    def save_scenario(self) -> None:
        self.write(GENERATOR.SCENARIO_DIR / f"{SCENARIO}.json", json.dumps({
            "id": SCENARIO, "workspace": "tests/fixtures/workspace", "formats": ["2.20"],
            "wire": self.wire,
        }))
        self.write(GENERATOR.MANIFEST_DIR / f"{SCENARIO}.steps",
                   "\n".join(step["tool"] for step in self.wire) + "\n")

    def generate(self, addresses: dict | None = None) -> dict:
        return GENERATOR.generate_report(self.repo, addresses, (SCENARIO,), (), ())

    def address_report(self) -> dict:
        self.write(GENERATOR.ROOT_TEST_FILE, f"fn {GENERATOR.ROOT_TEST}() {{}}")
        self.write("native/Configuration.xml", "<Configuration/>")
        rows = []
        for kind in ("Alpha", "Beta"):
            rows.append({"key": {
                "line": "8.3.27", "source_kind": "configuration", "branch": kind, "role": None,
                "owner_shape": [{"kind": "Configuration", "named": False, "role": None}],
            }, "witnesses": [{
                "owner": "main:Configuration", "at": f"main:{kind}", "level": "read",
                "origin": "native/Configuration.xml", "test_case": GENERATOR.ROOT_TEST, "reason": None,
            }]})
        return {"schema_version": 1, "revision": "fixture", "dirty": False, "scope": "fixture", "rows": rows}

    def read_fixture(self) -> list[dict]:
        return [
            {"tool": "unica.view", "args": {"at": "main:Beta"}, "expect": ["ok"],
             "assertions": [{"pointer": "/data/items/0/at", "eq": "main:Beta.ReadOnly"}],
             "captures": {"node": {"pointer": "/data/items/0/at"}}},
            {"tool": "unica.view", "args": {"at": "$capture.node"}, "expect": ["ok"],
             "assertions": [{"pointer": "/data/kind", "eq": "Beta"},
                            {"pointer": "/data/props/Comment", "eq": "Read only"}]},
        ]

    def generate_read(self, wire: list[dict]) -> dict:
        scenario = "S-Fixture-read"
        self.write(GENERATOR.SCENARIO_DIR / f"{scenario}.json", json.dumps({
            "id": scenario, "workspace": "tests/fixtures/workspace", "wire": wire,
        }))
        self.write(GENERATOR.MANIFEST_DIR / f"{scenario}.steps",
                   "\n".join(step["tool"] for step in wire) + "\n")
        return GENERATOR.generate_report(self.repo, None, (SCENARIO,), (
            (scenario, "Beta", len(wire), "/data/props/Comment"),
        ), check_selections=())

    def test_read_only_evidence_binds_address_and_field_without_changing_write(self) -> None:
        before = self.generate()
        for captured in (True, False):
            with self.subTest(captured=captured):
                wire = self.read_fixture()
                if not captured:
                    wire[-1]["args"]["at"] = "main:Beta.ReadOnly"
                    wire = [wire[-1]]
                report = self.generate_read(wire)
                for old, new in zip(before["rows"], report["rows"]):
                    for column in ("write", "address", "check"):
                        self.assertEqual(old[column], new[column])
                    for witness in old["read"]["evidence"]:
                        self.assertIn(witness, new["read"]["evidence"])
                beta = report["rows"][1]
                self.assertEqual(beta["read"]["status"], "evidenced")
                self.assertEqual(beta["write"]["status"], "no_evidence")
                item = beta["read"]["evidence"][0]
                self.assertEqual(item["at"], "main:Beta.ReadOnly")
                self.assertEqual(item["assertion"], wire[-1]["assertions"][1])
                self.assertIsNone(item["declared_formats"])
                if captured:
                    self.assertEqual(item["address_capture"]["producer_step"], 1)
                    self.assertEqual(item["address_capture"]["pointer"], "/data/items/0/at")
                else:
                    self.assertIsNone(item["address_capture"])

    def test_read_only_rejects_wrong_or_unconfirmed_captures(self) -> None:
        for damage in ("unknown", "pointer", "unasserted", "foreign-list", "shadowed"):
            with self.subTest(damage=damage):
                wire = self.read_fixture()
                if damage == "unknown":
                    wire[-1]["args"]["at"] = "$capture.missing"
                elif damage == "pointer":
                    wire[0]["captures"]["node"]["pointer"] = "/data/items/1/at"
                elif damage == "unasserted":
                    wire[0]["assertions"] = []
                elif damage == "foreign-list":
                    wire[0]["args"]["at"] = "main:Alpha"
                else:
                    wire.insert(1, {"tool": "unica.view", "args": {}, "expect": ["ok"],
                                    "captures": {"node": {"file": "source.xml", "kind": "sha256"}}})
                with self.assertRaises(ValueError):
                    self.generate_read(wire)

    def test_read_only_rejects_wrong_kind_and_missing_data_confirmation(self) -> None:
        for damage in ("address-kind", "asserted-kind", "child", "kind-only", "absent", "refusal"):
            with self.subTest(damage=damage):
                wire = self.read_fixture()
                if damage == "address-kind":
                    wire[-1]["args"]["at"] = "main:Alpha.ReadOnly"
                elif damage == "asserted-kind":
                    wire[-1]["assertions"][0]["eq"] = "Alpha"
                elif damage == "child":
                    wire[-1]["args"]["at"] = "main:Beta.ReadOnly.Props.Comment"
                elif damage == "kind-only":
                    wire[-1]["assertions"].pop()
                elif damage == "absent":
                    wire[-1]["assertions"][1] = {"pointer": "/data/props/Comment", "absent": True}
                else:
                    wire[-1]["expect"] = ["not_found"]
                with self.assertRaises(ValueError):
                    self.generate_read(wire)

    def check_fixture(self, outcome: str) -> dict:
        validator = "meta" if outcome == "warning" else "subsystem"
        if outcome == "warning":
            diagnostic = {"metadataPath": "Beta.Checked", "validator": validator,
                          "code": "validation_warning", "severity": "warning"}
            assertions = [{"pointer": "/data/diagnostics", "contains": diagnostic},
                          {"pointer": "/data/diagnostics", "eq": [deepcopy(diagnostic)]}]
        else:
            assertions = [{"pointer": "/data/at", "eq": "main:Beta.Checked"},
                          {"pointer": "/data/status", "eq": "passed"},
                          {"pointer": "/data/validators", "eq": [validator]}]
        return {"tool": "unica.check", "args": {"at": "main:Beta.Checked"}, "expect": ["ok"],
                "status": "passed", "validators": [validator], "assertions": assertions}

    def generate_check(self, step: dict, outcome: str) -> dict:
        scenario = "S-Fixture-check"
        self.write(GENERATOR.SCENARIO_DIR / f"{scenario}.json", json.dumps({
            "id": scenario, "workspace": "tests/fixtures/workspace", "wire": [step],
        }))
        self.write(GENERATOR.MANIFEST_DIR / f"{scenario}.steps", step["tool"] + "\n")
        validator = "meta" if outcome == "warning" else "subsystem"
        return GENERATOR.generate_report(self.repo, None, (SCENARIO,), (), (
            (scenario, "Beta", 1, "main:Beta.Checked", validator, outcome),
        ))

    def test_object_checks_keep_actual_assertions_and_warning_without_changing_other_columns(self) -> None:
        before = self.generate()
        for outcome in ("passed", "warning"):
            with self.subTest(outcome=outcome):
                step = self.check_fixture(outcome)
                report = self.generate_check(step, outcome)
                for old, new in zip(before["rows"], report["rows"]):
                    for column in ("read", "write", "address"):
                        self.assertEqual(old[column], new[column])
                beta = report["rows"][1]
                self.assertEqual(beta["check"]["status"], "evidenced")
                witness = beta["check"]["evidence"][0]
                self.assertEqual(witness["outcome"], outcome)
                self.assertEqual(witness["assertions"], step["assertions"])
                self.assertEqual(witness["at"], "main:Beta.Checked")
                self.assertEqual(witness["check_step"], 1)
                if outcome == "warning":
                    self.assertEqual(witness["assertions"][0]["contains"]["severity"], "warning")

    def test_check_requires_exact_object_result_and_validator_assertions(self) -> None:
        for field, value in (("args", {}), ("args", {"at": "main:Beta"}),
                             ("args", {"at": "main:Alpha.Checked"}),
                             ("tool", "unica.view"), ("expect", ["refused"]),
                             ("status", "readable"), ("validators", []),
                             ("assertions", [{"pointer": "/ok", "eq": True}])):
            with self.subTest(field=field, value=value):
                step = self.check_fixture("passed")
                step[field] = value
                with self.assertRaises(ValueError):
                    self.generate_check(step, "passed")
        for pointer in ("/data/at", "/data/status", "/data/validators"):
            with self.subTest(missing_assertion=pointer):
                step = self.check_fixture("passed")
                step["assertions"] = [a for a in step["assertions"] if a["pointer"] != pointer]
                with self.assertRaises(ValueError):
                    self.generate_check(step, "passed")
        step = self.check_fixture("passed")
        step["assertions"][0]["eq"] = "main:Alpha.Checked"
        with self.assertRaises(ValueError):
            self.generate_check(step, "passed")

    def test_check_warning_requires_object_validator_code_and_warning_severity(self) -> None:
        for field, value in (("metadataPath", "Alpha.Checked"), ("validator", "foreign"),
                             ("code", "validation_error"), ("severity", "error")):
            with self.subTest(field=field):
                step = self.check_fixture("warning")
                # The second assertion remains valid: an inconsistent foreign
                # diagnostic must not be rescued by another assertion.
                step["assertions"][0]["contains"][field] = value
                with self.assertRaises(ValueError):
                    self.generate_check(step, "warning")
        step = self.check_fixture("warning")
        step["assertions"] = []
        with self.assertRaises(ValueError):
            self.generate_check(step, "warning")
        with self.assertRaises(ValueError):
            self.generate_check(self.check_fixture("warning"), "passed")

    def test_all_declared_roots_have_rows_but_unknown_cells_stay_unknown(self) -> None:
        self.write("crates/unica-address/src/kinds/child.rs", 'KindDeclaration::new("Child")')
        report = self.generate()
        alpha, beta = report["rows"]
        self.assertEqual([alpha["kind"], beta["kind"]], ["Alpha", "Beta"])
        self.assertEqual(alpha["write"]["status"], "evidenced")
        self.assertEqual(alpha["read"]["evidence"][0]["view_step"], 3)
        for column in GENERATOR.COLUMNS:
            self.assertEqual(beta[column]["status"], "no_evidence")
        self.assertEqual(alpha["address"]["status"], "no_evidence")
        self.assertEqual(alpha["check"]["status"], "no_evidence")
        self.assertFalse(report["scope"]["complete"])

    def test_new_registration_rejects_a_saved_report_without_its_row(self) -> None:
        saved = self.generate()
        self.declare("gamma", "Gamma")
        with self.assertRaisesRegex(ValueError, "missing registered kind row: Gamma"):
            GENERATOR.validate_report(saved, self.generate())

    def test_duplicate_and_foreign_kind_rows_are_rejected(self) -> None:
        expected = self.generate()
        saved = deepcopy(expected)
        saved["rows"].append(deepcopy(saved["rows"][0]))
        with self.assertRaisesRegex(ValueError, "duplicate kind row: Alpha"):
            GENERATOR.validate_report(saved, expected)
        saved["rows"][-1]["kind"] = "Foreign"
        with self.assertRaisesRegex(ValueError, "unregistered kind row: Foreign"):
            GENERATOR.validate_report(saved, expected)

    def test_invalid_evidence_reference_is_rejected(self) -> None:
        expected = self.generate()
        for field, value in (("scenario", "missing.json"), ("view_step", 99), ("execute_step", 1)):
            with self.subTest(field=field):
                saved = deepcopy(expected)
                saved["rows"][0]["read"]["evidence"][0][field] = value
                with self.assertRaisesRegex(ValueError, "invalid or missing evidence"):
                    GENERATOR.validate_report(saved, expected)

    def test_preparation_without_execution_is_not_write_evidence(self) -> None:
        for token in ("$capture.foreign", None):
            with self.subTest(token=token):
                self.wire[1]["args"] = {"executionToken": token}
                self.save_scenario()
                with self.assertRaisesRegex(ValueError, "only prepared, not executed"):
                    self.generate()

    def test_refusal_unasserted_or_foreign_view_does_not_prove_read(self) -> None:
        original = deepcopy(self.wire)
        for field, value in (("expect", ["not_found"]), ("assertions", []),
                             ("args", {"at": "main:Beta.Probe"})):
            with self.subTest(field=field):
                self.wire = deepcopy(original)
                self.wire[2][field] = value
                self.save_scenario()
                with self.assertRaisesRegex(ValueError, "no asserted view"):
                    self.generate()

    def test_view_before_execution_does_not_prove_read(self) -> None:
        self.wire[1], self.wire[2] = self.wire[2], self.wire[1]
        self.save_scenario()
        with self.assertRaisesRegex(ValueError, "no asserted view"):
            self.generate()

    def test_step_manifest_mismatch_and_unregistered_creation_are_rejected(self) -> None:
        self.write(GENERATOR.MANIFEST_DIR / f"{SCENARIO}.steps", "unica.view\n")
        with self.assertRaisesRegex(ValueError, "invalid scenario/step manifest"):
            self.generate()
        self.wire[0]["args"]["ops"][0]["args"]["values"]["kind"] = "Foreign"
        self.save_scenario()
        with self.assertRaisesRegex(ValueError, "unregistered root kind"):
            self.generate()

    def test_address_scope_and_introduction_reason_are_retained_separately(self) -> None:
        address = self.address_report()
        witness = address["rows"][1]["witnesses"][0]
        witness.update(at=None, level=None, reason={"kind": "introduced_in", "line": "8.5.4"})
        report = self.generate(address)
        alpha, beta = report["rows"]
        item = alpha["address"]["evidence"][0]
        self.assertEqual(item["key"]["line"], "8.3.27")
        self.assertEqual(item["witness"]["level"], "read")
        self.assertEqual(beta["address"]["status"], "no_evidence")
        self.assertEqual(beta["address"]["absence_observations"][0]["witness"], witness)
        self.assertEqual(beta["read"]["status"], "no_evidence")
        self.assertEqual(alpha["check"]["status"], "no_evidence")
        GENERATOR.validate_report(report, self.generate(address))

    def test_broken_address_test_source_and_kind_links_are_rejected(self) -> None:
        original = self.address_report()
        for field, value in (("test_case", "missing_test"), ("origin", "missing.xml"),
                             ("at", "main:Beta.Foreign")):
            with self.subTest(field=field):
                address = deepcopy(original)
                address["rows"][0]["witnesses"][0][field] = value
                with self.assertRaises(ValueError):
                    self.generate(address)

    def test_other_native_root_producers_keep_their_test_reference(self) -> None:
        address = self.address_report()
        producer = GENERATOR.ADDRESS["FORM_TEMPLATE_SHARD"]
        source = "crates/unica-coder/src/infrastructure/v13_read/tests/nested/form.rs"
        self.write(source, f"fn {producer}() {{}}")
        address["rows"][0]["witnesses"][0]["test_case"] = producer
        report = self.generate(address)
        self.assertEqual(report["rows"][0]["address"]["evidence"][0]["test_file"], source)
        self.assertEqual(report["rows"][1]["read"]["status"], "no_evidence")

    def test_partial_duplicate_or_foreign_owner_address_rows_are_rejected(self) -> None:
        original = self.address_report()
        partial = deepcopy(original)
        partial["rows"].pop()
        with self.assertRaisesRegex(ValueError, "lacks root rows: Beta"):
            self.generate(partial)
        duplicate = deepcopy(original)
        duplicate["rows"].append(deepcopy(duplicate["rows"][0]))
        with self.assertRaisesRegex(ValueError, "duplicate root address row"):
            self.generate(duplicate)
        original["rows"][0]["witnesses"][0]["owner"] = "main:Catalog.Foreign"
        with self.assertRaisesRegex(ValueError, "another owner"):
            self.generate(original)

    def test_output_is_deterministic_under_address_row_reordering(self) -> None:
        address = self.address_report()
        before = json.dumps(self.generate(address), sort_keys=True)
        address["rows"].reverse()
        self.assertEqual(before, json.dumps(self.generate(address), sort_keys=True))

    def test_repository_scenarios_generate_valid_links_without_inventing_check_evidence(self) -> None:
        report = GENERATOR.generate_report()
        GENERATOR.validate_report(report, GENERATOR.generate_report())
        self.assertTrue(any(row["write"]["evidence"] for row in report["rows"]))
        check_kinds = {selection[1] for selection in GENERATOR.CHECK_SELECTIONS}
        self.assertEqual({row["kind"] for row in report["rows"]
                          if row["check"]["status"] == "evidenced"}, check_kinds)


if __name__ == "__main__":
    unittest.main()
