"""Shadow selection follows actual consumers; unknown inputs retain full CI."""
from __future__ import annotations
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import ci_shadow_selector as selector


def graph(revision):
    return {"revision": revision, "packages": {
        "unica-dcs": {"root": "crates/unica-dcs", "dependencies": [], "features": {}},
        "unica-dcs-designer": {"root": "crates/unica-dcs-designer", "dependencies": ["unica-dcs"], "features": {}},
        "unica-coder": {"root": "crates/unica-coder", "dependencies": ["unica-dcs-designer"], "features": {}},
        "unica-other": {"root": "crates/unica-other", "dependencies": [], "features": {}},
    }}


class ShadowSelectorTests(unittest.TestCase):
    def setUp(self):
        self.base, self.head = graph("base"), graph("head")
        self.change = {"status": "M", "path": "crates/unica-dcs/src/lib.rs"}

    def test_family_source_chooses_actual_reverse_consumers_and_keeps_full_mandatory(self):
        decision = selector.select(self.base, self.head, [self.change])
        self.assertEqual(decision["mode"], "affected")
        self.assertEqual(decision["packages"], ["unica-coder", "unica-dcs", "unica-dcs-designer"])
        self.assertEqual(decision["mandatoryMode"], "full")
        self.assertFalse(decision["affectedAdmission"])
        self.assertIn("package(=unica-dcs)", selector.filterset(decision))

    def test_base_graph_preserves_removed_dependency_edge(self):
        self.head["packages"]["unica-coder"]["dependencies"] = []
        self.assertIn("unica-coder", selector.select(self.base, self.head, [self.change])["packages"])
        del self.head["packages"]["unica-coder"]
        self.assertEqual(selector.select(self.base, self.head, [self.change])["mode"], "full")

    def test_mixed_groups_deletes_fixtures_specs_generators_locks_and_features_fall_back(self):
        inputs = ["Cargo.lock", "Cargo.toml", "plugins/unica/third-party/tools.lock.json", "tests/fixtures/source/8.3.27.xml", "plugins/unica/references/specs/schema.json", "scripts/ci/generate.py", "crates/unica-coder/src/composition.rs", "crates/unica-dcs/Cargo.toml"]
        for path in inputs:
            with self.subTest(path=path):
                decision = selector.select(self.base, self.head, [self.change, {"path": path, "status": "M"}])
                self.assertEqual(decision["mode"], "full")
        for status in ("D", "R100", "T", "unknown"):
            self.assertEqual(selector.select(self.base, self.head, [{**self.change, "status": status}])["mode"], "full")
        self.head["packages"]["unica-dcs"]["features"] = {"new": []}
        self.assertEqual(selector.select(self.base, self.head, [self.change])["mode"], "full")

    def test_missing_graph_and_unknown_owner_fail_closed(self):
        self.assertEqual(selector.select({}, self.head, [self.change])["mode"], "full")
        self.head["packages"]["unica-coder"]["dependencies"] = ["missing"]
        self.assertEqual(selector.select(self.base, self.head, [self.change])["mode"], "full")
        for decision in ({"mode": "affected", "mandatoryMode": "full", "affectedAdmission": False, "packages": []},
                         {"mode": "full", "mandatoryMode": "affected", "affectedAdmission": True}):
            with self.assertRaises(ValueError):
                selector.filterset(decision)


if __name__ == "__main__":
    unittest.main()
