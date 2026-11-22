"""Tests for the address coverage report's evidence boundaries."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[2] / "scripts/dev/generate-address-coverage.py"
SPEC = importlib.util.spec_from_file_location("generate_address_coverage", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)

ROOT_SHARD = "native_configuration_root_address_table_matches_published_branches"
MODULE_SHARD = "native_registered_modules_match_profile_and_physical_paths"
METADATA_SHARD = "native_metadata_address_table_matches_declared_and_observed_child_branches"
ROOT_KEYS = GENERATOR.ROOT_KEYS_SHARD
CHILD_KEYS = GENERATOR.METADATA_CHILD_KEYS_SHARD
MODULE_KEYS = GENERATOR.MODULE_ROLE_KEYS_SHARD


def root_witness(source: str, branch: str = "Catalog", *, introduced_in: str | None = None) -> dict:
    return {
        "schema_version": 1,
        "record_type": "evidence",
        "key": {
            "line": "8.3.27",
            "source_kind": "configuration",
            "owner_shape": [{"kind": "Configuration", "named": False, "role": None}],
            "branch": branch,
            "role": None,
        },
        "witness": {
            "owner": f"{source}:Configuration",
            "at": f"{source}:{branch}" if introduced_in is None else None,
            "level": "read" if introduced_in is None else None,
            "origin": "tests/dev/configuration.xml",
            "test_case": ROOT_SHARD,
            "reason": {"kind": "introduced_in", "line": introduced_in} if introduced_in else None,
        },
    }


def required_key(record: dict, *, introduced_in: str | None = None) -> dict:
    return {
        "schema_version": 1,
        "record_type": "required",
        "key": record["key"],
        "requirement": (
            {"kind": "introduced_in", "line": introduced_in}
            if introduced_in is not None else {"kind": "address"}
        ),
    }


def manager_witness(test_case: str = GENERATOR.DAEMON_SHARD) -> dict:
    record = root_witness("main", "Module")
    record["key"].update(
        owner_shape=[{"kind": "Document", "named": True, "role": None}], role="Manager",
    )
    record["witness"].update(
        owner="main:Document.Invoice",
        at="main:Document.Invoice.Module.Manager",
        level="roundtrip",
        test_case=test_case,
    )
    return record


class AddressCoverageReportTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))

    def write_shard(self, name: str, *records: dict) -> None:
        contents = "".join(json.dumps(record, ensure_ascii=False) + "\n" for record in records)
        (self.root / f"{name}.jsonl").write_text(contents, encoding="utf-8")

    def write_root_shard(self, *records: dict) -> None:
        self.write_shard(ROOT_SHARD, *records)

    def test_recursive_requirements_overlay_root_requirements_and_reject_a_missing_member(self) -> None:
        case = "native_recursive_subsystem_registrations_have_addresses"
        required_shard = "native_recursive_subsystem_requirements"
        root = root_witness("main", "Subsystem")
        branch = {**root, "witness": {**root["witness"], "test_case": case}}
        member = {**branch, "witness": {**branch["witness"], "at": "main:Subsystem.NewChild"}}
        required = required_key(root)
        required["requirement"] = {"kind": "read", "observations": [{
            "owner": root["witness"]["owner"], "origin": root["witness"]["origin"],
            "addresses": ["main:Subsystem", "main:Subsystem.NewChild"],
        }]}
        self.write_shard(ROOT_SHARD, root)
        self.write_shard(ROOT_KEYS, required_key(root))
        self.write_shard(required_shard, required)
        self.write_shard(case, branch)
        shards = frozenset({ROOT_SHARD, ROOT_KEYS, case, required_shard})
        with self.assertRaisesRegex(ValueError, r"missing required native projection evidence.*",):
            GENERATOR.merge_shards(self.root, shards)
        self.write_shard(case, branch, member)
        self.assertEqual(len(GENERATOR.merge_shards(self.root, shards)), 1)

    def test_special_projection_overlay_requires_the_declared_producer_and_origin(self) -> None:
        requirement_shard = "required_native_special_projection_keys"
        special = "native_special_projection_addresses_match_source"
        evidence = root_witness("main", "Item")
        evidence["key"]["owner_shape"] = [{"kind": "CommonForm", "named": True, "role": None}]
        witness = evidence["witness"]
        witness.update(owner="main:CommonForm.MainA", at="main:CommonForm.MainA.Item",
                       origin="tests/dev/form.xml", test_case=special)
        required = required_key(evidence)
        required["requirement"] = {"kind": "exact", "observations": [{
            "owner": witness["owner"], "origin": witness["origin"], "addresses": [witness["at"]],
            "level": "read", "test_case": GENERATOR.FORM_TEMPLATE_SHARD,
        }]}
        self.write_shard(requirement_shard, required)
        self.write_shard(special, evidence)
        self.write_shard(GENERATOR.FORM_TEMPLATE_SHARD, {
            **evidence, "witness": {**witness, "test_case": GENERATOR.FORM_TEMPLATE_SHARD, "origin": "tests/dev/other.xml"},
        })
        shards = frozenset({requirement_shard, special, GENERATOR.FORM_TEMPLATE_SHARD})
        with self.assertRaisesRegex(ValueError, r"missing required native projection evidence.*",):
            GENERATOR.merge_shards(self.root, shards)
        self.write_shard(GENERATOR.FORM_TEMPLATE_SHARD, {
            **evidence, "witness": {**witness, "test_case": GENERATOR.FORM_TEMPLATE_SHARD},
        })
        self.assertEqual(len(GENERATOR.merge_shards(self.root, shards)), 1)

    def test_external_special_projection_requires_a_form_owner_of_the_matching_source(self) -> None:
        special = GENERATOR.SPECIAL_PROJECTION_SHARD
        requirement_shard = GENERATOR.SPECIAL_PROJECTION_KEYS_SHARD
        evidence = root_witness("main", "Parameter")
        owner = "erf:ExternalReport.SyntheticReport.Form.Main"
        evidence["key"].update(source_kind="external_report", owner_shape=[
            {"kind": kind, "named": True, "role": None} for kind in ("ExternalReport", "Form")
        ])
        witness = evidence["witness"]
        witness.update(owner=owner, at=f"{owner}.Parameter", origin="tests/dev/form.xml", test_case=special)
        required = required_key(evidence)
        observation = {field: witness[field] for field in ("owner", "origin", "level", "test_case")}
        required["requirement"] = {"kind": "exact", "observations": [
            {**observation, "addresses": [witness["at"]]},
        ]}
        self.write_shard(special, evidence)
        self.write_shard(requirement_shard, required)
        shards = frozenset({special, requirement_shard})
        self.assertEqual(len(GENERATOR.merge_shards(self.root, shards)), 1)
        witness.update(owner=owner.replace("erf:", "epf:"), at=witness["at"].replace("erf:", "epf:"))
        self.write_shard(special, evidence)
        with self.assertRaisesRegex(ValueError, "external projection must identify its exact Form"):
            GENERATOR.merge_shards(self.root, shards)

    def test_named_dcs_requirements_reject_a_missing_or_flattened_child(self) -> None:
        shard = GENERATOR.DCS_STRUCTURE_KEYS_SHARD
        emitter = GENERATOR.DCS_STRUCTURE_SHARD
        rows = GENERATOR.native_dcs_structure_requirements(GENERATOR.REPO)
        self.assertEqual(len(rows), 9)
        requirements = {}
        groups = {}
        for row in rows:
            key = GENERATOR.structural_key(row["key"])
            expected = GENERATOR.projection_expectations(row, shard)
            self.assertEqual(len(expected), 2)
            requirements[(shard, key)] = expected
            groups[key] = {"key": row["key"], "witnesses": [
                {"test_case": test_case, "level": level, "owner": owner, "origin": origin,
                 "at": at, "reason": None} for test_case, level, owner, origin, at in sorted(expected)
            ]}
        GENERATOR.check_projection_requirements(requirements, groups)
        for group in groups.values():
            for witness in group["witnesses"]:
                if witness["at"].endswith(".Item.AddressProbeChild"):
                    self.assertEqual(witness["test_case"], emitter)
                    self.assertIn(".Item.AddressProbeParent.Item.", witness["at"])
                    witness["at"] = witness["at"].replace(".Item.AddressProbeParent", "")
        with self.assertRaisesRegex(ValueError, "missing required native projection evidence.*AddressProbeChild"):
            GENERATOR.check_projection_requirements(requirements, groups)

    def test_cross_line_common_role_requires_the_bare_module_and_not_its_body(self) -> None:
        shard = GENERATOR.CROSS_LINE_MODULE_KEYS_SHARD
        owner = "main:CommonModule.EventHandlers"
        record = required_key(root_witness("main", "Module"))
        record["key"].update(line="8.5.1", role="Common", owner_shape=[
            {"kind": "CommonModule", "named": True, "role": None},
        ])
        observation = {"owner": owner, "origin": "tests/dev/Module.bsl", "test_case": GENERATOR.MODULE_SHARD,
                       "level": "read"}
        record["requirement"] = {"kind": "exact", "observations": [{**observation, "addresses": [owner]}]}
        key = GENERATOR.structural_key(record["key"])
        required = {(shard, key): GENERATOR.projection_expectations(record, shard)}
        witness = {**observation, "at": f"{owner}.Body", "reason": None}
        with self.assertRaisesRegex(ValueError, "missing required native projection evidence.*CommonModule.EventHandlers"):
            GENERATOR.check_projection_requirements(required, {key: {"witnesses": [witness]}})
        witness["at"] = owner
        GENERATOR.check_projection_requirements(required, {key: {"witnesses": [witness]}})

    def test_implicit_native_document_number_requires_member_evidence_on_every_line(self) -> None:
        rows = GENERATOR.native_document_standard_attribute_requirements(GENERATOR.REPO)
        self.assertEqual({row["key"]["line"] for row in rows}, {"8.3.27", "8.5.1", "8.5.4"})
        shard = GENERATOR.DOCUMENT_STANDARD_ATTRIBUTE_KEYS_SHARD
        required = {}
        groups = {}
        for row in rows:
            key = GENERATOR.structural_key(row["key"])
            observations = GENERATOR.projection_expectations(row, shard)
            names = {address.rsplit(".", 1)[-1] for _, _, _, _, address in observations}
            self.assertEqual(names, {"StandardAttribute", "Posted", "Ref", "DeletionMark", "Date", "Number"})
            required[(shard, key)] = observations
            groups[key] = {"witnesses": [
                {"test_case": test_case, "level": level, "owner": owner, "origin": origin,
                 "at": at, "reason": None} for test_case, level, owner, origin, at in sorted(observations)
            ]}
        GENERATOR.check_projection_requirements(required, groups)
        group = next(group for key, group in groups.items() if key[0] == (8, 5, 4))
        group["witnesses"] = [witness for witness in group["witnesses"] if not witness["at"].endswith(".Number")]
        with self.assertRaisesRegex(ValueError, "missing required native projection evidence.*StandardAttribute.Number"):
            GENERATOR.check_projection_requirements(required, groups)

    def test_full_bsl_requirements_reject_missing_shard_and_mismatched_owner_or_address(self) -> None:
        shard = GENERATOR.BSL_PROJECTION_KEYS_SHARD
        daemon = GENERATOR.DAEMON_SHARD
        evidence = root_witness("main", "Variable")
        evidence["key"].update(owner_shape=[{"kind": "CommonModule", "named": True, "role": None}])
        owner = "main:CommonModule.Probe"
        at = f"{owner}.Variable"
        witness = evidence["witness"]
        witness.update(owner=owner, at=at, origin="tests/dev/Module.bsl", test_case=daemon)
        observation = {field: witness[field] for field in ("owner", "origin", "level", "test_case")}
        required = required_key(evidence)
        required["requirement"] = {"kind": "exact", "observations": [{**observation, "addresses": [at]}]}
        self.write_shard(daemon, evidence)
        shards = frozenset({shard, daemon})
        with self.assertRaisesRegex(ValueError, "evidence shards missing=.*required_bsl_projection_keys"):
            GENERATOR.merge_shards(self.root, shards)
        self.write_shard(shard, required)
        self.assertEqual(len(GENERATOR.merge_shards(self.root, shards)), 1)
        for field, value in (("owner", "main:CommonModule.Other"), ("at", f"{at}.Sample")):
            self.write_shard(daemon, {**evidence, "witness": {**witness, field: value}})
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "missing required native projection evidence"):
                GENERATOR.merge_shards(self.root, shards)

    def test_unnamed_dcs_rows_are_scoped_to_the_native_settings_collection(self) -> None:
        rows = GENERATOR.native_dcs_structure_requirements(GENERATOR.REPO)
        root = next(row for row in rows if row["key"]["line"] == "8.3.27"
                    and row["key"]["branch"] == "Item" and len(row["key"]["owner_shape"]) == 3)
        key = root["key"]
        observation = root["requirement"]["observations"][0]
        witness = {field: observation[field] for field in ("owner", "origin", "test_case", "level")}
        witness.update(at=observation["addresses"][0], reason=None)
        rows = [{"key": key, "witnesses": [witness, {**witness, "test_case": GENERATOR.FORM_TEMPLATE_SHARD}]}]
        matches = [item for item in GENERATOR.address_absence_observations(GENERATOR.REPO, rows)
                   if item["reason"]["kind"] == "unnamed_value_row"]
        self.assertEqual(len(matches), 1)
        self.assertNotIn("attempted_at", matches[0])

    def test_inherited_cfe_requires_each_registered_owner_and_local_declaration_origin(self) -> None:
        shard = GENERATOR.NATIVE_INHERITED_CFE_KEYS_SHARD
        rows = GENERATOR.native_inherited_cfe_registrations(GENERATOR.REPO)
        self.assertEqual(len(rows), 9)
        required = {}
        groups = {}
        for row in rows:
            key = GENERATOR.structural_key(row["key"])
            expected = GENERATOR.projection_expectations(row, shard)
            required[(shard, key)] = expected
            groups[key] = {"key": row["key"], "witnesses": [
                {"test_case": test_case, "level": level, "owner": owner, "origin": origin,
                 "at": at, "reason": None} for test_case, level, owner, origin, at in sorted(expected)
            ]}
        self.assertEqual(sum(map(len, required.values())), 20)
        GENERATOR.check_projection_requirements(required, groups)
        group = next(group for group in groups.values() if group["key"]["branch"] == "CommonTemplate")
        reasons = GENERATOR.address_absence_observations(GENERATOR.REPO, [group])
        self.assertIn("ext:CommonTemplate.ОбщийМакет.Area", [row["attempted_at"] for row in reasons
                                                            if row["reason"]["kind"] == "no_local_content"])
        member = next(witness for witness in group["witnesses"] if witness["at"].endswith(".ОбщийМакет"))
        member["origin"] = member["origin"].replace("/src/ext/", "/src/cf/")
        with self.assertRaisesRegex(ValueError, "missing required native projection evidence.*CommonTemplate.ОбщийМакет"):
            GENERATOR.check_projection_requirements(required, groups)

    def test_unnamed_characteristic_reason_is_scoped_to_confirmed_document(self) -> None:
        special = "native_special_projection_addresses_match_source"
        owner = "main:Document.АктОбУничтоженииПерсональныхДанных"
        key = {"line": "8.3.27", "source_kind": "configuration", "branch": "Characteristic", "role": None,
               "owner_shape": [{"kind": "Document", "named": True, "role": None}]}
        witness = {"owner": owner, "at": f"{owner}.Characteristic", "level": "read", "reason": None,
                   "origin": "tests/fixtures/acceptance/workspace/src/Documents/АктОбУничтоженииПерсональныхДанных.xml",
                   "test_case": special}
        rows = [{"key": key, "witnesses": [witness]},
                {"key": {**key, "line": "8.5.1"}, "witnesses": [witness]},
                {"key": key, "witnesses": [{**witness, "test_case": METADATA_SHARD}]}]
        result = [row for row in GENERATOR.address_absence_observations(GENERATOR.REPO, rows)
                  if row["reason"]["kind"] == "unnamed_value_row"]
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0]["key"], key)
        self.assertNotIn("attempted_at", result[0])

    def test_address_absence_keeps_named_probes_out_of_positive_rows(self) -> None:
        rows = [{"key": root_witness("main", introduced_in="8.5.1")["key"],
                 "witnesses": [root_witness("main", introduced_in="8.5.1")["witness"]]}]
        before = json.dumps(rows, ensure_ascii=False)
        observations = GENERATOR.address_absence_observations(GENERATOR.REPO, rows)
        self.assertEqual(json.dumps(rows, ensure_ascii=False), before)
        counts = {}
        for case in observations:
            reason = case["reason"]["kind"]
            counts[reason] = counts.get(reason, 0) + 1
            if reason == "not_subordinate":
                self.assertEqual(case["key"]["owner_shape"][0]["kind"], "Sequence")
                self.assertEqual(case["evidence"]["named_probes"]["named_module"], "not_found")
            if reason == "unverified":
                self.assertIn(case["key"]["owner_shape"][0]["kind"], {"Constant", "DocumentNumerator"})
                self.assertIn("ChildObjects", case["evidence"]["diagnostic"])
            self.assertNotIn("at", case)
        self.assertEqual(counts, {"introduced_in": 1, "not_subordinate": 12, "unverified": 24})

    def test_container_refusal_cannot_be_promoted_to_proven_nonsubordination(self) -> None:
        source = GENERATOR.REPO / GENERATOR.NATIVE_ABSENCE_FIXTURE
        fixture = self.root / GENERATOR.NATIVE_ABSENCE_FIXTURE
        fixture.parent.mkdir(parents=True)
        data = json.loads(source.read_text(encoding="utf-8"))
        case = next(case for case in data["probes"] if case["owner"].startswith("main:Constant."))
        case["stages"] = {"import": 0, "apply": 0, "export": 0}
        case["export"] = next(case["export"] for case in data["probes"] if "export" in case)
        fixture.write_text(json.dumps(data, ensure_ascii=False), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "child subordination"):
            GENERATOR.address_absence_observations(self.root, [])

    def test_local_form_companion_absence_keeps_the_declaration_proof(self) -> None:
        required = GENERATOR.native_cfe_ownership_requirements(GENERATOR.REPO)
        rows = []
        for case in required:
            witnesses = [
                {"owner": obs["owner"], "at": obs["addresses"][0], "origin": obs["origin"],
                 "level": obs["level"], "test_case": GENERATOR.CFE_OWNERSHIP_SHARD, "reason": None}
                for obs in case["requirement"]["observations"]
            ]
            rows.append({"key": case["key"], "witnesses": witnesses})
        before = json.dumps(rows, ensure_ascii=False)
        cases = GENERATOR.address_absence_observations(GENERATOR.REPO, rows)
        inherited = [case for case in cases if case["reason"]["kind"] == "no_local_content"]
        self.assertEqual(len(inherited), 3)
        for case in inherited:
            self.assertEqual(case["key"]["role"], "Form")
            self.assertEqual(case["evidence"]["declaration"], "ext:Catalog.CorpusCatalog.Form.CorpusForm.Metadata")
            self.assertTrue(case["evidence"]["absent_local_file"].endswith("/Ext/Form.xml"))
        self.assertEqual(json.dumps(rows, ensure_ascii=False), before)
        markdown = GENERATOR.render_markdown({
            "revision": "test", "dirty": False, "scope": "", "rows": rows, "absence_observations": cases,
        })
        self.assertIn("Address absence observations", markdown)
        self.assertIn("No local content", markdown)
        self.assertIn("ext:Catalog.CorpusCatalog.Form.CorpusForm.Metadata (roundtrip)", markdown)

    def test_empty_branch_gap_requires_direct_reads_without_claiming_tree_enumeration(self) -> None:
        pairs = (
            ("FilterCriterion", "Form"), ("FilterCriterion", "Command"),
            ("SettingsStorage", "Form"), ("SettingsStorage", "Template"),
            ("HTTPService", "URLTemplate"), ("WebService", "Operation"),
        )
        records = []
        for line in ("8.3.27", "8.5.1", "8.5.4"):
            for kind, branch in pairs:
                record = root_witness("main", branch)
                record["key"].update(
                    line=line, owner_shape=[{"kind": kind, "named": True, "role": None}],
                )
                owner = f"main:{kind}.Probe"
                record["witness"].update(
                    owner=owner, at=f"{owner}.{branch}", test_case=METADATA_SHARD,
                )
                records.append(record)

        self.write_shard(METADATA_SHARD, *records)
        rows = GENERATOR.merge_shards(self.root, frozenset({METADATA_SHARD}))
        GENERATOR.mark_empty_branch_enumeration_gaps(rows)
        self.assertEqual(len(rows), 18)
        for row in rows:
            self.assertEqual(row["enumeration_gap"]["issue"], 2224)
            self.assertEqual(row["witnesses"][0]["level"], "read")
        observations = GENERATOR.address_absence_observations(GENERATOR.REPO, rows)
        gaps = [case for case in observations if case["reason"]["kind"] == "enumeration_gap"]
        self.assertEqual(len(gaps), 18)
        self.assertTrue(all(case["evidence"]["direct_read"] for case in gaps))
        report = GENERATOR.render_markdown({
            "revision": "test", "dirty": False, "scope": "", "rows": rows, "absence_observations": observations,
        })
        self.assertIn("main:FilterCriterion.Probe.Form (read)", report)
        self.assertIn("Empty branch enumeration", report)
        self.assertIn("Owner enumeration gap [#2224]", report)

        missing, others = records[0], records[1:]
        self.write_shard(METADATA_SHARD, *others)
        rows = GENERATOR.merge_shards(self.root, frozenset({METADATA_SHARD}))
        with self.assertRaisesRegex(ValueError, r"8.3.27.*FilterCriterion.Form"):
            GENERATOR.mark_empty_branch_enumeration_gaps(rows)

        missing["witness"]["at"] = "main:FilterCriterion.Probe.Command"
        self.write_shard(METADATA_SHARD, missing, *others)
        rows = GENERATOR.merge_shards(self.root, frozenset({METADATA_SHARD}))
        with self.assertRaisesRegex(ValueError, r"8.3.27.*FilterCriterion.Form"):
            GENERATOR.mark_empty_branch_enumeration_gaps(rows)

    def test_native_subsystem_interface_requirements_guard_existing_and_new_branches(self) -> None:
        shard = GENERATOR.SUBSYSTEM_INTERFACE_KEYS_SHARD
        cases = GENERATOR.native_subsystem_interface_requirements(GENERATOR.REPO)
        self.assertEqual(len(cases), 12)
        self.assertEqual({row["key"]["branch"] for row in cases}, {"Interface", "Command", "Group", "Subsystem"})

        def witnessed(required: dict) -> dict:
            at = required["requirement"]["at"]
            return {
                "schema_version": 1, "record_type": "evidence", "key": required["key"],
                "witness": {
                    "owner": at.rsplit(".", 1)[0], "at": at, "level": "read", "reason": None,
                    "origin": required["requirement"]["origin"], "test_case": GENERATOR.DAEMON_SHARD,
                },
            }

        evidence = [witnessed(row) for row in cases]
        required_shards = frozenset({shard, GENERATOR.DAEMON_SHARD})
        self.write_shard(shard, *cases)
        self.write_shard(GENERATOR.DAEMON_SHARD, *evidence)
        self.assertEqual(len(GENERATOR.merge_shards(self.root, required_shards)), 12)

        for branch in ("Group", "Subsystem"):
            lost = next(
                row for row in evidence if row["key"]["line"] == "8.5.1" and row["key"]["branch"] == branch
            )
            self.write_shard(GENERATOR.DAEMON_SHARD, *[row for row in evidence if row != lost])
            with self.assertRaisesRegex(
                ValueError, rf"(?s)missing required native subsystem interface coverage.*Interface\.{branch}"
            ):
                GENERATOR.merge_shards(self.root, required_shards)

        fixture_repo = Path(self.enterContext(tempfile.TemporaryDirectory()))
        for origin in {row["requirement"]["origin"] for row in cases}:
            relative = Path(origin)
            paths = (
                relative,
                relative.parents[2] / "Подсистема1.xml",
                relative.parents[3] / "Configuration.xml",
                relative.parents[1] / "Subsystems/Подсистема2.xml",
            )
            for path in paths:
                target = fixture_repo / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes((GENERATOR.REPO / path).read_bytes())
        relative = Path(lost["witness"]["origin"])
        config_path = fixture_repo / relative.parents[3] / "Configuration.xml"
        configuration = ET.parse(config_path)
        md = configuration.getroot().tag.removesuffix("MetaDataObject")
        registrations = configuration.find(f"{md}Configuration/{md}ChildObjects")
        ET.SubElement(registrations, md + "Subsystem").text = "SecondSubsystem"
        configuration.write(config_path, encoding="utf-8", xml_declaration=True)
        descriptor = ET.parse(fixture_repo / relative.parents[2] / "Подсистема1.xml")
        second_owner = descriptor.find(md + "Subsystem")
        second_owner.set("uuid", "00000000-0000-4000-8000-000000000001")
        second_owner.find(f"{md}Properties/{md}Name").text = "SecondSubsystem"
        second_file = fixture_repo / relative.parents[2] / "SecondSubsystem.xml"
        descriptor.write(second_file, encoding="utf-8", xml_declaration=True)
        second_interface = second_file.with_suffix("") / "Ext/CommandInterface.xml"
        second_interface.parent.mkdir(parents=True)
        second_interface.write_bytes((fixture_repo / relative).read_bytes())
        self.assertEqual(GENERATOR.native_subsystem_interface_requirements(fixture_repo), cases)

        target = fixture_repo / relative
        document = ET.parse(target)
        root = document.getroot()
        namespace = root.tag.removesuffix("CommandInterface")
        order = root.find(namespace + "SubsystemsOrder")
        self.assertIsNotNone(order)
        order[0].text = "Subsystem.Подсистема1.Subsystem.Probe"
        document.write(target, encoding="utf-8", xml_declaration=True)
        with self.assertRaisesRegex(ValueError, "SubsystemsOrder requires a registered direct child"):
            GENERATOR.native_subsystem_interface_requirements(fixture_repo)
        root.remove(order)
        document.write(target, encoding="utf-8", xml_declaration=True)
        shortened = GENERATOR.native_subsystem_interface_requirements(fixture_repo)
        self.assertEqual(
            [row for row in cases if row not in shortened],
            [row for row in cases if row["key"]["line"] == "8.5.1" and row["key"]["branch"] == "Subsystem"],
        )


    def test_registered_native_query_requires_exact_read_evidence_in_both_modes(self) -> None:
        shard = GENERATOR.DCS_QUERY_KEYS_SHARD
        emitter = GENERATOR.FORM_TEMPLATE_SHARD
        cases = GENERATOR.native_dcs_query_requirements(GENERATOR.REPO)
        self.assertEqual(
            {(case["key"]["line"], case["key"]["owner_shape"][0]["kind"]) for case in cases},
            {(line, kind) for line in ("8.3.27", "8.5.1", "8.5.4") for kind in ("DataProcessor", "Report")},
        )
        self.assertEqual(len(cases), 6)
        evidence = [
            {
                "schema_version": 1, "record_type": "evidence", "key": case["key"],
                "witness": {
                    "owner": case["requirement"]["at"].rsplit(".", 1)[0],
                    "at": case["requirement"]["at"], "level": "read", "reason": None,
                    "origin": case["requirement"]["origin"], "test_case": emitter,
                },
            }
            for case in cases
        ]
        query_origin = GENERATOR.REPO / cases[-1]["requirement"]["origin"]
        parse = ET.parse

        def parse_without_last_query(path: Path) -> ET.ElementTree:
            document = parse(path)
            if path == query_origin:
                root = document.getroot()
                namespace = root.tag.removesuffix("DataCompositionSchema")
                dataset = root.find(namespace + "dataSet")
                dataset.remove(dataset.find(namespace + "query"))
            return document

        with patch.object(GENERATOR.ET, "parse", side_effect=parse_without_last_query):
            before_query = GENERATOR.native_dcs_query_requirements(GENERATOR.REPO)
        self.assertEqual(before_query, cases[:-1])
        required_shards = frozenset({shard, emitter})
        for native_only in (False, True):
            with self.subTest(native_only=native_only):
                self.write_shard(shard, *before_query)
                self.write_shard(emitter, *evidence[:-1])
                self.assertEqual(len(GENERATOR.merge_shards(self.root, required_shards, native_only=native_only)), 5)
                self.write_shard(shard, *cases)
                with self.assertRaisesRegex(ValueError, r"missing required native DCS Query coverage"):
                    GENERATOR.merge_shards(self.root, required_shards, native_only=native_only)
                self.write_shard(emitter, *evidence)
                self.assertEqual(len(GENERATOR.merge_shards(self.root, required_shards, native_only=native_only)), 6)
                wrong = {**evidence[-1], "witness": {**evidence[-1]["witness"], "level": "roundtrip"}}
                self.write_shard(emitter, *evidence[:-1], wrong)
                with self.assertRaisesRegex(ValueError, r"missing required native DCS Query coverage"):
                    GENERATOR.merge_shards(self.root, required_shards, native_only=native_only)

    def test_native_role_right_uses_registered_xml_objects(self) -> None:
        cases = GENERATOR.native_role_right_requirements(GENERATOR.REPO)
        self.assertEqual({row["key"]["line"] for row in cases}, {"8.3.27", "8.5.1", "8.5.4"})
        for row in cases:
            self.assertEqual(row["key"]["branch"], "Right")
            self.assertEqual(row["requirement"]["owner"], "main:Role.SalesReader")
            self.assertEqual(set(row["requirement"]["addresses"]), {
                "main:Role.SalesReader.Right",
                "main:Role.SalesReader.Right.Catalog_Валюты",
                "main:Role.SalesReader.Right.Document_Orders",
            })

        role_file = GENERATOR.REPO / cases[-1]["requirement"]["origin"]
        parse = ET.parse

        def parse_with_one_rights_object(path: Path) -> ET.ElementTree:
            document = parse(path)
            if path == role_file:
                rights = document.getroot()
                namespace = rights.tag.removesuffix("Rights")
                rights.remove(rights.findall(namespace + "object")[-1])
            return document

        with patch.object(GENERATOR.ET, "parse", side_effect=parse_with_one_rights_object):
            filtered = GENERATOR.native_role_right_requirements(GENERATOR.REPO)
        self.assertEqual(filtered[:-1], cases[:-1])
        self.assertEqual(set(filtered[-1]["requirement"]["addresses"]), {
            "main:Role.SalesReader.Right", "main:Role.SalesReader.Right.Catalog_Валюты",
        })

    def test_native_role_right_requires_exact_branch_and_members_with_other_witnesses_present(self) -> None:
        shard = GENERATOR.ROLE_RIGHT_KEYS_SHARD
        cases = GENERATOR.native_role_right_requirements(GENERATOR.REPO)
        evidence = [
            {
                "schema_version": 1, "record_type": "evidence", "key": row["key"],
                "witness": {
                    "owner": row["requirement"]["owner"], "at": address, "level": "read", "reason": None,
                    "origin": row["requirement"]["origin"], "test_case": METADATA_SHARD,
                },
            }
            for row in cases for address in row["requirement"]["addresses"]
        ]
        lost = next(row for row in evidence if row["witness"]["at"].endswith(".Document_Orders"))
        daemon = {
            **lost, "witness": {
                "owner": "main:Role.Reader", "at": "main:Role.Reader.Right",
                "level": "read", "reason": None, "test_case": GENERATOR.DAEMON_SHARD,
                "origin": "tests/fixtures/acceptance/workspace-role-comment-subsystem-help/src/Roles/Reader/Ext/Rights.xml",
            },
        }
        rest = [row for row in evidence if row is not lost]
        required_shards = frozenset({shard, METADATA_SHARD, GENERATOR.DAEMON_SHARD})
        self.write_shard(shard, *cases)
        self.write_shard(GENERATOR.DAEMON_SHARD, daemon)
        for native_only in (False, True):
            with self.subTest(native_only=native_only):
                self.write_shard(METADATA_SHARD, *evidence)
                self.assertEqual(len(GENERATOR.merge_shards(self.root, required_shards, native_only=native_only)), 3)
                for replacement in (
                    [],
                    [{**lost, "witness": {**lost["witness"], "origin": cases[-1]["requirement"]["origin"]}}],
                    [{**lost, "witness": {**lost["witness"], "level": "roundtrip"}}],
                ):
                    self.write_shard(METADATA_SHARD, *rest, *replacement)
                    with self.assertRaisesRegex(ValueError, r"(?s)missing required native Role.Right coverage.*Document_Orders.*8\.3\.27"):
                        GENERATOR.merge_shards(self.root, required_shards, native_only=native_only)

    def test_native_xdto_requires_each_witness_from_its_own_line_and_package(self) -> None:
        shard = GENERATOR.XDTO_KEYS_SHARD
        required = GENERATOR.native_xdto_requirements(GENERATOR.REPO)
        self.assertEqual(len(required), 15)
        observations = [o for row in required for o in row["requirement"]["observations"]]
        self.assertEqual(len(observations), 21)
        self.assertEqual(sum(len(o["addresses"]) - 1 for o in observations), 30)
        evidence = [
            {
                "schema_version": 1, "record_type": "evidence", "key": row["key"],
                "witness": {
                    "owner": o["owner"], "at": at, "origin": o["origin"],
                    "level": "read", "reason": None, "test_case": METADATA_SHARD,
                },
            }
            for row in required for o in row["requirement"]["observations"] for at in o["addresses"]
        ]
        target = next(
            row for row in evidence if row["key"]["line"] == "8.5.1"
            and row["witness"]["at"].endswith(".Type.EnterpriseData.Property.Версия")
        )
        rest = [row for row in evidence if row != target]
        self.assertTrue(any(row["key"] == target["key"] for row in rest))
        wrong_line = {**target, "key": {**target["key"], "line": "8.5.4"}}
        wrong_origin = target["witness"]["origin"].replace("/2.21/", "/2.22/")
        self.assertEqual(
            (GENERATOR.REPO / target["witness"]["origin"]).read_bytes(),
            (GENERATOR.REPO / wrong_origin).read_bytes(),
        )
        wrong_package = {**target, "witness": {**target["witness"], "origin": wrong_origin}}
        wrong_level = {**target, "witness": {**target["witness"], "level": "roundtrip"}}
        self.write_shard(shard, *required)
        required_shards = frozenset({shard, METADATA_SHARD})
        for native_only in (False, True):
            with self.subTest(native_only=native_only):
                self.write_shard(METADATA_SHARD, *evidence)
                self.assertEqual(len(GENERATOR.merge_shards(self.root, required_shards, native_only=native_only)), 15)
                for replacement in ([], [wrong_line], [wrong_package], [wrong_level]):
                    self.write_shard(METADATA_SHARD, *rest, *replacement)
                    with self.assertRaisesRegex(
                        ValueError, r"(?s)missing required native XDTO coverage.*Property\.Версия \(8\.5\.1\)",
                    ):
                        GENERATOR.merge_shards(self.root, required_shards, native_only=native_only)

    def test_native_subsystem_content_requires_exact_relation_and_collection_reads(self) -> None:
        shard = GENERATOR.SUBSYSTEM_CONTENT_KEYS_SHARD
        required = GENERATOR.native_subsystem_content_requirements(GENERATOR.REPO)
        self.assertEqual(len(required), 12)
        evidence = [
            {
                "schema_version": 1, "record_type": "evidence", "key": row["key"],
                "witness": {
                    "owner": o["owner"], "at": at, "origin": o["origin"],
                    "level": "read", "reason": None, "test_case": METADATA_SHARD,
                },
            }
            for row in required for o in row["requirement"]["observations"] for at in o["addresses"]
        ]
        self.assertEqual(len([row for row in evidence if row["key"]["branch"] == "Relation"]), 6)
        self.assertEqual(len([row for row in evidence if row["key"]["branch"] == "Subsystem"]), 9)
        relation = next(
            row for row in evidence if row["key"]["line"] == "8.5.1" and row["key"]["branch"] == "Relation"
            and len(row["key"]["owner_shape"]) == 2
        )
        collection_member = next(
            row for row in evidence if row["key"]["line"] == "8.5.1"
            and row["witness"]["at"].endswith(".Subsystem.БазоваяФункциональность")
        )
        self.assertTrue(any(
            row != collection_member and row["key"] == collection_member["key"] for row in evidence
        ))
        wrong_line = relation["witness"]["origin"].replace("/2.21/", "/2.22/")
        wrong_origin = collection_member["witness"]["origin"]
        self.write_shard(shard, *required)
        shards = frozenset({shard, METADATA_SHARD})
        for native_only in (False, True):
            with self.subTest(native_only=native_only):
                self.write_shard(METADATA_SHARD, *evidence)
                self.assertEqual(len(GENERATOR.merge_shards(self.root, shards, native_only=native_only)), 12)
                for target, replacement in (
                    (collection_member, {}),
                    (relation, {"origin": wrong_line}),
                    (relation, {"origin": wrong_origin}),
                    (relation, {"level": "roundtrip"}),
                ):
                    rest = [row for row in evidence if row != target]
                    if replacement:
                        rest.append({**target, "witness": {**target["witness"], **replacement}})
                    self.write_shard(METADATA_SHARD, *rest)
                    with self.assertRaisesRegex(
                        ValueError, r"(?s)missing required native subsystem content coverage.*\(8\.5\.1\)",
                    ):
                        GENERATOR.merge_shards(self.root, shards, native_only=native_only)

    def test_native_wsdl_requires_a_roundtrip_from_the_same_line_and_coder_shard(self) -> None:
        shard = GENERATOR.WSDL_KEYS_SHARD
        required = [
            {
                "schema_version": 1, "record_type": "required",
                "key": {
                    "line": line, "source_kind": "configuration",
                    "owner_shape": [{"kind": "WSReference", "named": True, "role": None}],
                    "branch": "Definition", "role": None,
                },
                "requirement": {"kind": "roundtrip", "observations": [{
                    "owner": "main:WSReference.Probe",
                    "origin": f"tests/fixtures/formats/platform_xml/{version}/all_kinds/src/WSReferences/Probe/Ext/WSDefinition.xml",
                    "addresses": ["main:WSReference.Probe.Definition"],
                }]},
            }
            for version, line in (("2.20", "8.3.27"), ("2.21", "8.5.1"), ("2.22", "8.5.4"))
        ]
        evidence = [
            {
                "schema_version": 1, "record_type": "evidence", "key": row["key"],
                "witness": {
                    "owner": observation["owner"], "origin": observation["origin"],
                    "at": at, "level": "roundtrip", "reason": None, "test_case": METADATA_SHARD,
                },
            }
            for row in required for observation in row["requirement"]["observations"]
            for at in observation["addresses"]
        ]
        self.assertEqual(GENERATOR.native_wsdl_requirements(GENERATOR.REPO), required)
        before, target, after = evidence
        foreign_origin = after["witness"]["origin"]
        self.assertEqual(
            (GENERATOR.REPO / target["witness"]["origin"]).read_bytes(),
            (GENERATOR.REPO / foreign_origin).read_bytes(),
        )
        self.write_shard(shard, *required)
        shards = frozenset({shard, METADATA_SHARD, GENERATOR.FORM_TEMPLATE_SHARD})
        alternative = {**target, "witness": {**target["witness"], "test_case": GENERATOR.FORM_TEMPLATE_SHARD}}
        self.write_shard(GENERATOR.FORM_TEMPLATE_SHARD, alternative)
        for native_only in (False, True):
            with self.subTest(native_only=native_only):
                self.write_shard(METADATA_SHARD, *evidence)
                self.assertEqual(len(GENERATOR.merge_shards(self.root, shards, native_only=native_only)), 3)
                for changes in ({}, {"level": "read"}, {"origin": foreign_origin}):
                    with self.subTest(changes=changes):
                        substitute = [{**target, "witness": {**target["witness"], **changes}}] if changes else []
                        self.write_shard(METADATA_SHARD, before, after, *substitute)
                        with self.assertRaisesRegex(
                            ValueError, r"(?s)missing required native WSDL coverage.*Definition \(8\.5\.1\): expected roundtrip",
                        ):
                            GENERATOR.merge_shards(self.root, shards, native_only=native_only)

    def test_native_mxl_requires_each_registered_rows_read_provenance(self) -> None:
        shard = GENERATOR.MXL_KEYS_SHARD
        producer = GENERATOR.FORM_TEMPLATE_SHARD
        required = GENERATOR.native_mxl_requirements(GENERATOR.REPO, native_only=True)
        self.assertEqual(
            [(row["key"]["line"], row["key"]["branch"], len(row["requirement"]["observations"]))
             for row in required],
            [("8.5.1", "Area", 1), ("8.5.1", "Body", 3), ("8.5.1", "Parameter", 3),
             ("8.5.4", "Area", 1), ("8.5.4", "Body", 3), ("8.5.4", "Parameter", 3)],
        )
        owner = "main:Report.АнализВерсийОбъектов.Template.МакетОтчета"
        for row in required:
            branch = row["key"]["branch"]
            expected = [f"{owner}.Area"] if branch == "Area" else [
                f"{owner}.Area.{name}.{branch}" for name in ("Подвал", "Строка", "Шапка")
            ]
            self.assertEqual([at for o in row["requirement"]["observations"] for at in o["addresses"]],
                             expected)
        evidence = self.mxl_evidence(required, producer)
        target_at = f"{owner}.Area.Строка.Parameter"
        target = next(record for record in evidence
                      if record["key"]["line"] == "8.5.1" and record["witness"]["at"] == target_at)
        foreign = next(record for record in evidence
                       if record["key"]["line"] == "8.5.4" and record["witness"]["at"] == target_at)
        alternative = {**target, "witness": {**target["witness"], "test_case": METADATA_SHARD}}
        self.write_shard(shard, *required)
        self.write_shard(METADATA_SHARD, alternative)
        shards = frozenset({shard, producer, METADATA_SHARD})
        for native_only in (False, True):
            self.write_shard(producer, *evidence)
            self.assertEqual(len(GENERATOR.merge_shards(self.root, shards, native_only=native_only)), 6)
            for replacement in ({}, {"origin": foreign["witness"]["origin"]}, {"level": "roundtrip"},
                                {"owner": target["witness"]["owner"].replace("Строка", "Шапка")}):
                with self.subTest(native_only=native_only, replacement=replacement):
                    remaining = [record for record in evidence if record is not target]
                    if replacement:
                        remaining.append({**target, "witness": {**target["witness"], **replacement}})
                    self.write_shard(producer, *remaining)
                    with self.assertRaisesRegex(
                        ValueError, r"(?s)missing required native MXL coverage.*Строка\.Parameter \(8\.5\.1\)",
                    ):
                        GENERATOR.merge_shards(self.root, shards, native_only=native_only)

    def test_native_mxl_platform_export_requires_native_origin_only_in_full_mode(self) -> None:
        shard = GENERATOR.MXL_KEYS_SHARD
        producer = GENERATOR.DAEMON_SHARD
        native = GENERATOR.native_mxl_requirements(GENERATOR.REPO, native_only=True)
        full = GENERATOR.native_mxl_requirements(GENERATOR.REPO, native_only=False)
        self.assertEqual([row for row in full if row["key"]["line"] != "8.3.27"], native)
        platform = [row for row in full if row["key"]["line"] == "8.3.27"]
        self.assertEqual([row["key"]["branch"] for row in platform], ["Area", "Body", "Parameter"])
        evidence = self.mxl_evidence(platform, producer)
        self.assertEqual(len(evidence), 7)
        target = next(record for record in evidence if record["key"]["branch"] == "Area")
        self.assertEqual(target["witness"]["at"], "cf:Report.F06Report.Template.PlatformExport.Area")
        synthetic = {**target, "witness": {
            **target["witness"],
            "owner": target["witness"]["owner"].replace("PlatformExport", "F06Template"),
            "at": target["witness"]["at"].replace("PlatformExport", "F06Template"),
            "origin": target["witness"]["origin"].replace("PlatformExport", "F06Template"),
        }}
        self.write_shard(shard, *platform)
        self.write_shard(producer, *evidence, synthetic)
        shards = frozenset({shard, producer})
        self.assertEqual(len(GENERATOR.merge_shards(self.root, shards)), 3)
        self.write_shard(producer, *(record for record in evidence if record is not target), synthetic)
        with self.assertRaisesRegex(
            ValueError, r"(?s)missing required native MXL coverage.*PlatformExport\.Area \(8\.3\.27\)",
        ):
            GENERATOR.merge_shards(self.root, shards)

    @staticmethod
    def mxl_evidence(requirements: list[dict], producer: str) -> list[dict]:
        return [
            {"schema_version": 1, "record_type": "evidence", "key": row["key"], "witness": {
                "owner": observation["owner"], "origin": observation["origin"], "at": at,
                "level": "read", "reason": None, "test_case": producer,
            }}
            for row in requirements for observation in row["requirement"]["observations"]
            for at in observation["addresses"]
        ]



    def test_native_schedule_keeps_descriptor_read_and_requires_companion_roundtrip(self) -> None:
        shard = GENERATOR.SCHEDULE_KEYS_SHARD
        owner = "main:ScheduledJob.KeyProbe"
        address = f"{owner}.Schedule"
        required = GENERATOR.native_schedule_requirements(GENERATOR.REPO)
        self.assertEqual({row["key"]["line"] for row in required}, {"8.3.27", "8.5.1", "8.5.4"})
        evidence = []
        for row in required:
            observation, = row["requirement"]["observations"]
            self.assertEqual(observation["owner"], owner)
            self.assertEqual(observation["addresses"], [address])
            self.assertTrue(observation["descriptor_origin"].endswith("/ScheduledJobs/KeyProbe.xml"))
            self.assertTrue(observation["origin"].endswith("/Schedule.xml"))
            for level, origin in (("read", observation["descriptor_origin"]),
                                  ("roundtrip", observation["origin"])):
                evidence.append({
                    "schema_version": 1, "record_type": "evidence", "key": row["key"],
                    "witness": {"owner": owner, "at": address, "origin": origin, "level": level,
                                "test_case": METADATA_SHARD, "reason": None},
                })
        earlier_read, target = [row for row in evidence if row["key"]["line"] == "8.5.1"]
        foreign = next(row for row in evidence
                       if row["key"]["line"] == "8.5.4" and row["witness"]["level"] == "roundtrip")
        alternate = {**target, "witness": {**target["witness"], "test_case": GENERATOR.FORM_TEMPLATE_SHARD}}
        self.write_shard(shard, *required)
        self.write_shard(GENERATOR.FORM_TEMPLATE_SHARD, alternate)
        shards = frozenset({shard, METADATA_SHARD, GENERATOR.FORM_TEMPLATE_SHARD})
        for native_only in (False, True):
            self.write_shard(METADATA_SHARD, *evidence)
            self.assertEqual(len(GENERATOR.merge_shards(self.root, shards, native_only=native_only)), len(required))
            for omitted, changes in (
                (target, {}), (target, {"level": "read"}),
                (target, {"origin": foreign["witness"]["origin"]}), (earlier_read, {}),
            ):
                with self.subTest(native_only=native_only, omitted=omitted["witness"]["level"], changes=changes):
                    remaining = [record for record in evidence if record is not omitted]
                    if changes:
                        remaining.append({**omitted, "witness": {**omitted["witness"], **changes}})
                    self.write_shard(METADATA_SHARD, *remaining)
                    with self.assertRaisesRegex(
                        ValueError, rf"(?s)missing required native Schedule coverage.*Schedule \(8\.5\.1\)"
                        rf": expected {omitted['witness']['level']}",
                    ):
                        GENERATOR.merge_shards(self.root, shards, native_only=native_only)



    def test_native_cfe_ownership_requires_exact_read_and_physical_roundtrip_evidence(self) -> None:
        producer = GENERATOR.CFE_OWNERSHIP_SHARD
        shard = GENERATOR.CFE_OWNERSHIP_KEYS_SHARD
        required = GENERATOR.native_cfe_ownership_requirements(GENERATOR.REPO)
        self.assertEqual(len(required), 24)
        for line in ("8.3.27", "8.5.1", "8.5.4"):
            self.assertEqual(sum(row["key"]["line"] == line for row in required), 8)
        evidence = [
            {
                "schema_version": 1, "record_type": "evidence", "key": row["key"],
                "witness": {"owner": obs["owner"], "origin": obs["origin"], "at": at,
                            "level": obs["level"], "reason": None, "test_case": producer},
            }
            for row in required for obs in row["requirement"]["observations"] for at in obs["addresses"]
        ]
        self.assertEqual(len(evidence), 39)
        inherited = [item for item in evidence if "/inherited/" in item["witness"]["origin"]]
        self.assertEqual(len(inherited), 6)
        inherited_form = next(item for item in inherited if item["key"]["line"] == "8.5.1"
                              and item["witness"]["at"].endswith(".CorpusForm"))
        inherited_metadata = next(item for item in inherited if item["key"]["line"] == "8.5.4"
                                  and item["witness"]["at"].endswith(".CorpusForm.Metadata"))
        own_body = next(item for item in evidence if item["key"]["line"] == "8.5.1"
                        and item["witness"]["at"].endswith("OwnProbe.Body"))
        adopted = next(item for item in evidence if item["key"]["line"] == "8.5.4"
                       and item["witness"]["at"].endswith("CorpusForm.Metadata"))
        self.write_shard(shard, *required)
        shards = frozenset({producer, shard})
        for native_only in (False, True):
            self.write_shard(producer, *evidence)
            self.assertEqual(len(GENERATOR.merge_shards(self.root, shards, native_only=native_only)), 24)
            for target, mutation, error in (
                (own_body, None, "OwnProbe.Body.*expected read"),
                (inherited_form, None, r"CorpusForm \(8\.5\.1\): expected read from .*/inherited/"),
                (inherited_metadata, None, r"CorpusForm.Metadata \(8\.5\.4\): expected roundtrip from .*/inherited/"),
                (own_body, {"level": "roundtrip"}, "OwnProbe.Body.*cannot round-trip"),
                (adopted, {"level": "read"}, "CorpusForm.Metadata.*expected roundtrip"),
                (adopted, {"origin": adopted["witness"]["origin"].replace("/adopted/", "/cf/")},
                 "CorpusForm.Metadata.*expected roundtrip"),
                (adopted, {"origin": adopted["witness"]["origin"].replace("-2.22/", "-2.21/")},
                 "CorpusForm.Metadata.*expected roundtrip"),
                (adopted, {"owner": adopted["witness"]["owner"].replace("ext:", "main:"),
                           "at": adopted["witness"]["at"].replace("ext:", "main:")},
                 "CorpusForm.Metadata.*expected roundtrip"),
            ):
                with self.subTest(native_only=native_only, mutation=mutation):
                    remaining = [row for row in evidence if row is not target]
                    if mutation is not None:
                        remaining.append({**target, "witness": {**target["witness"], **mutation}})
                    self.write_shard(producer, *remaining)
                    with self.assertRaisesRegex(
                        ValueError, rf"(?s)^missing required native CFE ownership coverage:.*{error}"
                    ):
                        GENERATOR.merge_shards(self.root, shards, native_only=native_only)

    def test_native_cfe_ownership_is_derived_from_registration_links(self) -> None:
        parent = Path("tests/fixtures/acceptance")
        for version in ("2.20", "2.21", "2.22"):
            fixture = parent / f"native-cfe-ownership-{version}"
            shutil.copytree(GENERATOR.REPO / fixture, self.root / fixture)
        fixture = parent / "native-cfe-ownership-2.22"
        manifest_out = self.root / fixture / "manifest.json"
        manifest = json.loads(manifest_out.read_bytes())
        name = "adopted/Catalogs/CorpusCatalog/Forms/CorpusForm.xml"
        descriptor = self.root / fixture / name
        data = descriptor.read_bytes()
        link = b"3a1f0000-0000-4000-8000-000000001982"
        self.assertIn(link, data)
        data = data.replace(link, b"3a1f0000-0000-4000-8000-000000001981", 1)
        descriptor.write_bytes(data)
        with self.assertRaisesRegex(ValueError, "fixture bytes differ"):
            GENERATOR.native_cfe_ownership_requirements(self.root)
        manifest["files"][name] = {"sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}
        manifest_out.write_text(json.dumps(manifest), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "adopted reference does not match parent descriptor"):
            GENERATOR.native_cfe_ownership_requirements(self.root)

    def test_native_adopted_extension_language_requires_both_read_origins(self) -> None:
        producer = GENERATOR.EXTENSION_LANGUAGE_SHARD
        shard = GENERATOR.EXTENSION_LANGUAGE_KEYS_SHARD
        required = GENERATOR.native_adopted_extension_language_requirements(GENERATOR.REPO)
        self.assertEqual([row["key"]["line"] for row in required], ["8.3.27", "8.5.1", "8.5.4"])
        evidence = [
            {
                "schema_version": 1, "record_type": "evidence", "key": row["key"],
                "witness": {"owner": obs["owner"], "origin": obs["origin"], "at": at, "level": "read",
                            "reason": None, "test_case": producer},
            }
            for row in required for obs in row["requirement"]["observations"] for at in obs["addresses"]
        ]
        self.assertEqual([row["witness"]["at"] for row in evidence], ["ext:Language", "ext:Language.Русский"] * 3)
        target = evidence[3]
        witness = target["witness"]
        self.write_shard(shard, *required)
        self.write_shard(ROOT_SHARD, root_witness("main", "Language"))
        self.write_shard(GENERATOR.DAEMON_SHARD, {
            **target, "witness": {**witness, "test_case": GENERATOR.DAEMON_SHARD},
        })
        shards = frozenset({shard, producer, ROOT_SHARD, GENERATOR.DAEMON_SHARD})
        diagnostic = (
            r"(?s)missing required native adopted extension Language coverage.*"
            r"ext:Language\.Русский \(8\.5\.1\): expected read"
        )
        for native_only in (False, True):
            self.write_shard(producer, *evidence)
            self.assertEqual(len(GENERATOR.merge_shards(self.root, shards, native_only=native_only)), 4)
            for replacement in (
                None, {"owner": "main:Configuration", "at": "main:Language.Русский"},
                {"origin": witness["origin"].replace("/2.21/", "/2.22/")}, {"level": "roundtrip"},
            ):
                with self.subTest(native_only=native_only, replacement=replacement):
                    remaining = [item for item in evidence if item is not target]
                    if replacement is not None:
                        remaining.append({**target, "witness": {**witness, **replacement}})
                    self.write_shard(producer, *remaining)
                    with self.assertRaisesRegex(ValueError, diagnostic):
                        GENERATOR.merge_shards(self.root, shards, native_only=native_only)
            self.write_shard(producer, *[item for item in evidence if item is not evidence[0]])
            with self.assertRaisesRegex(ValueError, r"ext:Language \(8\.3\.27\): expected read"):
                GENERATOR.merge_shards(self.root, shards, native_only=native_only)

    def test_native_adopted_language_does_not_infer_belonging_from_registration_name(self) -> None:
        required = GENERATOR.native_adopted_extension_language_requirements(GENERATOR.REPO)
        for row in required:
            for obs in row["requirement"]["observations"]:
                origin = Path(obs["origin"])
                target = self.root / origin
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes((GENERATOR.REPO / origin).read_bytes())
        origin = required[-1]["requirement"]["observations"][1]["origin"]
        target = self.root / origin
        native = target.read_bytes()
        tag = b"<ObjectBelonging>Adopted</ObjectBelonging>"
        self.assertIn(tag, native)
        target.write_bytes(native.replace(tag, b"", 1))
        with self.assertRaisesRegex(ValueError, "expected registered adopted Language"):
            GENERATOR.native_adopted_extension_language_requirements(self.root)
        uuid = b"00000000-0000-0000-0000-000000000001"
        self.assertIn(uuid, native)
        target.write_bytes(native.replace(uuid, b"00000000-0000-0000-0000-000000000000", 1))
        with self.assertRaisesRegex(ValueError, "missing extended configuration object UUID"):
            GENERATOR.native_adopted_extension_language_requirements(self.root)

    def test_external_inline_requires_exact_descriptor_read_for_root_and_nested_keys(self) -> None:
        shard = GENERATOR.EXTERNAL_INLINE_KEYS_SHARD
        producer = GENERATOR.EXTERNAL_ROOT_SHARD
        required = GENERATOR.native_external_inline_requirements(GENERATOR.REPO)
        self.assertEqual(len(required), 24)
        self.assertEqual(
            {(row["key"]["line"], row["key"]["source_kind"], row["key"]["branch"],
              tuple(segment["kind"] for segment in row["key"]["owner_shape"])) for row in required},
            {(line, source, branch, shape)
             for line in ("8.3.27", "8.5.1", "8.5.4")
             for source, kind in (("external_processor", "ExternalDataProcessor"),
                                  ("external_report", "ExternalReport"))
             for branch, shape in (("Attribute", (kind,)), ("TabularSection", (kind,)),
                                   ("Attribute", (kind, "TabularSection")),
                                   ("StandardAttribute", (kind, "TabularSection")))},
        )
        evidence = self.external_inline_evidence(required)
        self.assertEqual(len(evidence), 72)
        expected_at = "erf:ExternalReport.SyntheticReport.TabularSection.Lines.StandardAttribute.LineNumber"
        target = next(item for item in evidence if item["key"]["line"] == "8.5.1"
                      and item["witness"]["at"] == expected_at)
        witness = target["witness"]
        alternate = {**target, "witness": {**witness, "test_case": GENERATOR.DAEMON_SHARD}}
        false_roundtrip = {**target, "witness": {**witness, "level": "roundtrip"}}
        self.write_shard(shard, *required)
        self.write_shard(GENERATOR.DAEMON_SHARD, alternate)
        shards = frozenset({shard, producer, GENERATOR.DAEMON_SHARD})
        missing = [item for item in evidence if item is not target]
        for native_only in (False, True):
            self.write_shard(producer, *evidence)
            self.assertEqual(len(GENERATOR.merge_shards(self.root, shards, native_only=native_only)), 24)
            for records, error in (
                (missing, "expected read"),
                ([*missing, {**target, "witness": {**witness, "origin": witness["origin"].replace("/2.21/", "/2.22/")}}],
                 "expected read"),
                ([*missing, {**target, "witness": {**witness, "owner": witness["owner"].replace(".Lines", ".Other")}}],
                 "expected read"),
                ([*missing, false_roundtrip], "inline resource cannot round-trip"),
                ([*evidence, false_roundtrip], "inline resource cannot round-trip"),
            ):
                with self.subTest(native_only=native_only, error=error):
                    self.write_shard(producer, *records)
                    with self.assertRaisesRegex(
                        ValueError,
                        r"(?s)missing required native external inline coverage.*"
                        rf"StandardAttribute\.LineNumber \(8\.5\.1\): {error}",
                    ):
                        GENERATOR.merge_shards(self.root, shards, native_only=native_only)

    @staticmethod
    def external_inline_evidence(required: list[dict]) -> list[dict]:
        return [
            {
                "schema_version": 1, "record_type": "evidence", "key": row["key"],
                "witness": {"owner": obs["owner"], "at": at, "origin": obs["origin"],
                            "level": "read", "test_case": GENERATOR.EXTERNAL_ROOT_SHARD, "reason": None},
            }
            for row in required for obs in row["requirement"]["observations"] for at in obs["addresses"]
        ]

    def test_external_inline_new_registration_cannot_be_hidden_by_collection_read(self) -> None:
        producer = GENERATOR.EXTERNAL_ROOT_SHARD
        shard = GENERATOR.EXTERNAL_INLINE_KEYS_SHARD
        required = GENERATOR.native_external_inline_requirements(GENERATOR.REPO)
        for origin in {
            Path(obs["origin"]) for row in required for obs in row["requirement"]["observations"]
        }:
            for path in (origin, origin.parents[1] / "manifest.json"):
                copy = self.root / path
                copy.parent.mkdir(parents=True, exist_ok=True)
                copy.write_bytes((GENERATOR.REPO / path).read_bytes())

        # Alter only a provisional test copy. The real expected registrations
        # always come from the previously authenticated, versioned native files.
        parent = self.root / "tests/fixtures/formats/platform_xml/2.22/external_artifacts"
        descriptor = parent / "epf/SyntheticProcessor.xml"
        data = descriptor.read_bytes().replace(b"<Name>Item</Name>", b"<Name>GateProbe</Name>", 1)
        descriptor.write_bytes(data)
        with self.assertRaisesRegex(ValueError, "unexpected descriptor bytes"):
            GENERATOR.native_external_inline_requirements(self.root)
        manifest_path = parent / "manifest.json"
        manifest = json.loads(manifest_path.read_bytes())
        manifest["files"]["epf/SyntheticProcessor.xml"] = {
            "sha256": hashlib.sha256(data).hexdigest(), "size": len(data),
        }
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        mutated = GENERATOR.native_external_inline_requirements(self.root)
        evidence = self.external_inline_evidence(required)
        shard_root = self.root / "shards"
        shard_root.mkdir()
        for name, records in ((producer, evidence), (shard, mutated)):
            (shard_root / f"{name}.jsonl").write_text(
                "".join(f"{json.dumps(record, ensure_ascii=False)}\n" for record in records),
                encoding="utf-8",
            )
        with self.assertRaisesRegex(
            ValueError,
            r"(?s)external inline coverage.*TabularSection\.Lines\.Attribute\.GateProbe \(8\.5\.4\): expected read",
        ):
            GENERATOR.merge_shards(self.root / "shards", frozenset({producer, shard}))

    def test_witnesses_for_one_shape_are_grouped_in_stable_order(self) -> None:
        first, second = root_witness("main"), root_witness("other")
        self.write_root_shard(second, first)
        result = GENERATOR.merge_shards(self.root, frozenset({ROOT_SHARD}))
        self.assertEqual(result, [{
            "key": first["key"],
            "witnesses": [first["witness"], second["witness"]],
        }])
        self.write_root_shard(first, second)
        self.assertEqual(GENERATOR.merge_shards(self.root, frozenset({ROOT_SHARD})), result)

    def test_missing_required_shard_fails_before_report(self) -> None:
        self.write_root_shard(root_witness("main"))
        with self.assertRaisesRegex(ValueError, f"missing=.*{MODULE_SHARD}"):
            GENERATOR.merge_shards(self.root, frozenset({ROOT_SHARD, MODULE_SHARD}))

    def test_duplicate_witness_is_rejected(self) -> None:
        record = root_witness("main")
        self.write_root_shard(record, record)
        with self.assertRaisesRegex(ValueError, "duplicate evidence record"):
            GENERATOR.merge_shards(self.root, frozenset({ROOT_SHARD}))

    def test_read_and_absence_cannot_share_a_structural_key(self) -> None:
        self.write_root_shard(root_witness("main", "PaletteColor"), root_witness("other", "PaletteColor", introduced_in="8.5.1"))
        with self.assertRaisesRegex(ValueError, "conflicting address evidence"):
            GENERATOR.merge_shards(self.root, frozenset({ROOT_SHARD}))

    def test_existing_evidence_shard_cannot_hide_a_missing_key(self) -> None:
        self.write_root_shard(root_witness("main"))
        self.write_shard(ROOT_KEYS, required_key(root_witness("main", "Document")))
        with self.assertRaisesRegex(ValueError, r'(?s)1 address, 0 introduced_in.*"branch":"Document".*no native evidence'):
            GENERATOR.merge_shards(self.root, frozenset({ROOT_SHARD, ROOT_KEYS}))

    def test_daemon_example_cannot_replace_a_native_witness_for_the_same_key(self) -> None:
        catalog = root_witness("main")
        catalog["witness"]["test_case"] = GENERATOR.DAEMON_SHARD
        self.write_shard(GENERATOR.DAEMON_SHARD, catalog)
        self.write_shard(ROOT_KEYS, required_key(catalog))
        with self.assertRaisesRegex(ValueError, "no native evidence"):
            GENERATOR.merge_shards(self.root, frozenset({GENERATOR.DAEMON_SHARD, ROOT_KEYS}))

    def test_required_introduction_must_match_the_witness(self) -> None:
        palette = root_witness("main", "PaletteColor", introduced_in="8.5.4")
        self.write_root_shard(palette)
        self.write_shard(ROOT_KEYS, required_key(palette, introduced_in="8.5.1"))
        with self.assertRaisesRegex(ValueError, r'(?s)0 address, 1 introduced_in.*expected .*8\.5\.1.*found .*8\.5\.4'):
            GENERATOR.merge_shards(self.root, frozenset({ROOT_SHARD, ROOT_KEYS}))

    def test_required_keys_leave_the_v1_report_unchanged(self) -> None:
        catalog = root_witness("main")
        palette = root_witness("main", "PaletteColor", introduced_in="8.5.1")
        child = root_witness("main", "Attribute")
        child["key"]["owner_shape"] = [{"kind": "Catalog", "named": True, "role": None}]
        child["witness"].update(
            owner="main:Catalog.Items", at="main:Catalog.Items.Attribute", test_case=METADATA_SHARD,
        )
        self.write_root_shard(catalog, palette)
        self.write_shard(METADATA_SHARD, child)
        evidence_shards = frozenset({ROOT_SHARD, METADATA_SHARD})
        original = GENERATOR.merge_shards(self.root, evidence_shards)
        self.write_shard(ROOT_KEYS, required_key(catalog), required_key(palette, introduced_in="8.5.1"))
        self.write_shard(CHILD_KEYS, required_key(child))
        self.assertEqual(
            GENERATOR.merge_shards(self.root, evidence_shards | {ROOT_KEYS, CHILD_KEYS}),
            original,
        )

    def test_child_introduction_is_refused(self) -> None:
        child = root_witness("main", "Attribute")
        child["key"]["owner_shape"] = [{"kind": "Catalog", "named": True, "role": None}]
        self.write_shard(CHILD_KEYS, required_key(child, introduced_in="8.5.1"))
        with self.assertRaisesRegex(ValueError, "only valid for a configuration root kind"):
            GENERATOR.merge_shards(self.root, frozenset({CHILD_KEYS}))

    def test_external_form_requirement_rejects_a_missing_native_witness(self) -> None:
        form = root_witness("erf", "Form")
        form["key"].update(
            source_kind="external_report",
            owner_shape=[{"kind": "ExternalReport", "named": True, "role": None}],
        )
        form["witness"].update(
            owner="erf:ExternalReport.SyntheticReport",
            at="erf:ExternalReport.SyntheticReport.Form",
            test_case=GENERATOR.EXTERNAL_ROOT_SHARD,
        )
        shard = GENERATOR.EXTERNAL_CHILD_KEYS_SHARD
        self.write_shard(shard, required_key(form))
        self.write_shard(GENERATOR.EXTERNAL_ROOT_SHARD, form)
        self.assertEqual(
            GENERATOR.merge_shards(self.root, frozenset({shard, GENERATOR.EXTERNAL_ROOT_SHARD})),
            [{"key": form["key"], "witnesses": [form["witness"]]}],
        )
        command = {
            **form,
            "key": {**form["key"], "branch": "Command"},
            "witness": {
                **form["witness"],
                "at": "erf:ExternalReport.SyntheticReport.Command",
            },
        }
        self.write_shard(GENERATOR.EXTERNAL_ROOT_SHARD, command)
        with self.assertRaisesRegex(ValueError, r'(?s)missing required native coverage.*"branch":"Form"'):
            GENERATOR.merge_shards(self.root, frozenset({shard, GENERATOR.EXTERNAL_ROOT_SHARD}))

    def test_duplicate_and_conflicting_required_keys_are_refused(self) -> None:
        palette = root_witness("main", "PaletteColor", introduced_in="8.5.4")
        first = required_key(palette, introduced_in="8.5.4")
        for second, diagnostic in [(first, "duplicate"), (required_key(palette), "conflicting")]:
            with self.subTest(diagnostic=diagnostic):
                self.write_shard(ROOT_KEYS, first, second)
                with self.assertRaisesRegex(ValueError, f"{diagnostic} requirement"):
                    GENERATOR.merge_shards(self.root, frozenset({ROOT_KEYS}))

    def test_required_module_role_accepts_a_daemon_or_native_witness(self) -> None:
        manager = manager_witness()
        self.write_root_shard(root_witness("main"))
        self.write_shard(MODULE_KEYS, required_key(manager))
        with self.assertRaisesRegex(
            ValueError, r'(?s)missing required configuration module roles: 1 address.*"role":"Manager"',
        ):
            GENERATOR.merge_shards(self.root, frozenset({ROOT_SHARD, MODULE_KEYS}))
        for shard, level in [(GENERATOR.DAEMON_SHARD, "roundtrip"), (MODULE_SHARD, "read")]:
            with self.subTest(shard=shard):
                witness = manager_witness(shard)
                witness["witness"]["level"] = level
                self.write_shard(shard, witness)
                rows = GENERATOR.merge_shards(self.root, frozenset({ROOT_SHARD, MODULE_KEYS, shard}))
                self.assertIn({"key": witness["key"], "witnesses": [witness["witness"]]}, rows)
                (self.root / f"{shard}.jsonl").unlink()

    def test_native_only_requires_and_validates_module_file_without_requiring_witnesses(self) -> None:
        catalog = root_witness("main")
        self.write_root_shard(catalog)
        self.write_shard(ROOT_KEYS, required_key(catalog))
        shards = frozenset({ROOT_SHARD, ROOT_KEYS, MODULE_KEYS})
        with self.assertRaisesRegex(ValueError, f"missing=.*{MODULE_KEYS}"):
            GENERATOR.merge_shards(self.root, shards, native_only=True)

        manager = manager_witness()
        self.write_shard(MODULE_KEYS, required_key(manager))
        self.assertEqual(
            GENERATOR.merge_shards(self.root, shards, native_only=True),
            [{"key": catalog["key"], "witnesses": [catalog["witness"]]}],
        )
        self.write_shard(ROOT_KEYS, required_key(root_witness("main", "Document")))
        with self.assertRaisesRegex(ValueError, "missing required native coverage: 1 address"):
            GENERATOR.merge_shards(self.root, shards, native_only=True)

        manager["key"]["line"] = "8.5.1"
        self.write_shard(MODULE_KEYS, required_key(manager))
        with self.assertRaisesRegex(ValueError, "must name an 8.3.27 Module role"):
            GENERATOR.merge_shards(self.root, shards, native_only=True)

    def test_eds_module_requires_native_roundtrip_in_both_report_modes(self) -> None:
        role = root_witness("main", "Module")
        role["key"].update(
            line="8.5.4",
            owner_shape=[
                {"kind": kind, "named": True, "role": None}
                for kind in ("ExternalDataSource", "Cube", "DimensionTable")
            ],
            role="Object",
        )
        owner = "main:ExternalDataSource.Source.Cube.Sales.DimensionTable.Products"
        role["witness"].update(
            owner=owner, at=f"{owner}.Module.Object", level="roundtrip", test_case=METADATA_SHARD,
        )
        branch = {
            **role,
            "key": {**role["key"], "role": None},
            "witness": {**role["witness"], "at": f"{owner}.Module", "level": "read"},
        }
        role_required = {**required_key(role), "requirement": {"kind": "roundtrip"}}
        branch_required = {**required_key(branch), "requirement": {"kind": "read"}}
        shard = GENERATOR.EDS_MODULE_KEYS_SHARD
        self.write_shard(shard, role_required, branch_required)
        shards = frozenset({shard, METADATA_SHARD})
        for native_only in (False, True):
            with self.subTest(native_only=native_only):
                self.write_shard(METADATA_SHARD, branch, role)
                rows = GENERATOR.merge_shards(self.root, shards, native_only=native_only)
                self.assertEqual(len(rows), 2)

                self.write_shard(METADATA_SHARD, branch)
                with self.assertRaisesRegex(ValueError, r'(?s)"role":"Object".*expected roundtrip, found no native evidence'):
                    GENERATOR.merge_shards(self.root, shards, native_only=native_only)

                read_only = {**role, "witness": {**role["witness"], "level": "read"}}
                self.write_shard(METADATA_SHARD, branch, read_only)
                with self.assertRaisesRegex(ValueError, r'(?s)"role":"Object".*expected roundtrip, found read'):
                    GENERATOR.merge_shards(self.root, shards, native_only=native_only)

    def test_module_role_cannot_declare_or_witness_absence(self) -> None:
        manager = manager_witness(MODULE_SHARD)
        absence = required_key(manager, introduced_in="8.5.1")
        self.write_shard(MODULE_KEYS, absence)
        with self.assertRaisesRegex(ValueError, "module role requirement"):
            GENERATOR.merge_shards(self.root, frozenset({MODULE_KEYS}), native_only=True)
        manager["witness"].update(at=None, level=None, reason=absence["requirement"])
        self.write_shard(MODULE_SHARD, manager)
        with self.assertRaisesRegex(ValueError, "must verify a positive address"):
            GENERATOR.merge_shards(self.root, frozenset({MODULE_KEYS, MODULE_SHARD}))


if __name__ == "__main__":
    unittest.main()
