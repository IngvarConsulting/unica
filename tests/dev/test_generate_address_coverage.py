"""Tests for the address coverage report's evidence boundaries."""

from __future__ import annotations

import importlib.util
import json
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
