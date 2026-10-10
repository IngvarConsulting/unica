from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
HISTORICAL_SKILLS = {"dcs-compile", "dcs-edit"}
REMOVED_SKILLS = HISTORICAL_SKILLS | {
    "dcs-decompile", "skd-compile", "skd-edit", "skd-decompile"
}


class DcsNamingContractTests(unittest.TestCase):
    def test_mutations_use_canonical_operations_without_legacy_tools(self) -> None:
        source = REPO_ROOT / "crates" / "unica-coder" / "src"
        tool_registry = (source / "application" / "mod.rs").read_text(encoding="utf-8")
        legacy_surface = re.findall(r'name: "(unica\.(?:dcs|skd)\.[^"]+)"', tool_registry)
        self.assertEqual(legacy_surface, [])

        apply_registry = (source / "domain" / "apply.rs").read_text(encoding="utf-8")
        registered = set(re.findall(r'\(\s*"([^"]+)"\s*,\s*Dcs\s*,', apply_registry))
        writer = (source / "infrastructure" / "native_operations" / "dcs_primitives.rs").read_text(encoding="utf-8")
        implemented = set(re.findall(r'"([^"]+)"\s*=>\s*Self::[A-Za-z]+', writer))
        self.assertEqual(registered, implemented)
        self.assertTrue({
            "dataSource.add", "dataSet.add", "field.add", "fieldRole.set",
            "query.set", "parameter.add", "variant.add", "selection.add",
            "filter.add", "order.add", "structure.add", "dataSetLink.add",
        } <= registered)
        self.assertNotIn("dcs.set", registered)

    def test_dcs_guidance_retires_dcs_and_skd_prompt_skills(self) -> None:
        skill_root = REPO_ROOT / "plugins" / "unica" / "skills"
        skill_names = {path.name for path in skill_root.iterdir() if path.is_dir()}
        self.assertTrue(REMOVED_SKILLS.isdisjoint(skill_names))

    def test_provenance_tracks_active_operations_and_preserves_donor_paths(self) -> None:
        path = REPO_ROOT / "docs" / "provenance" / "skill-upstreams.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        entries = {
            entry["skill"]: entry
            for upstream in data["upstreams"]
            for entry in upstream["entries"]
        }
        self.assertTrue(HISTORICAL_SKILLS <= entries.keys())
        self.assertTrue({"skd-compile", "skd-edit"}.isdisjoint(entries.keys()))
        primitive_path = "crates/unica-coder/src/infrastructure/native_operations/dcs_primitives.rs"
        for skill in HISTORICAL_SKILLS:
            entry = entries[skill]
            self.assertFalse(entry["promptSkill"])
            self.assertIn(primitive_path, entry["localPaths"])
            self.assertIn("tests/fixtures/acceptance/scenario-corpus.json", entry["contractPaths"])
            for active_path in entry["localPaths"] + entry["contractPaths"]:
                self.assertTrue((REPO_ROOT / active_path).exists(), active_path)
            self.assertTrue(
                any("skd" in donor.lower() for donor in entry["upstreamPaths"]),
                f"{skill} must retain its verbatim donor path",
            )

    def test_platform_schema_compatibility_spellings_remain_unchanged(self) -> None:
        contracts = (
            REPO_ROOT
            / "crates"
            / "unica-coder"
            / "src"
            / "application"
            / "tool_contracts.rs"
        ).read_text(encoding="utf-8")

        self.assertIn('"SetMainSKD"', contracts)
        self.assertIn('"setMainSKD"', contracts)
        self.assertNotIn('"SetMainDCS"', contracts)
        self.assertNotIn('"setMainDCS"', contracts)


if __name__ == "__main__":
    unittest.main()
