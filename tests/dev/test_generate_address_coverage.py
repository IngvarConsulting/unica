"""Tests for the address coverage report's evidence boundaries."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


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
