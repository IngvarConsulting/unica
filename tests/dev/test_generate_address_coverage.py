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


class AddressCoverageReportTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))

    def write_root_shard(self, *records: dict) -> None:
        contents = "".join(json.dumps(record, ensure_ascii=False) + "\n" for record in records)
        (self.root / f"{ROOT_SHARD}.jsonl").write_text(contents, encoding="utf-8")

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


if __name__ == "__main__":
    unittest.main()
