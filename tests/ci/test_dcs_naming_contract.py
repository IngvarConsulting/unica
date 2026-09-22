from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
EXPECTED_TOOLS = {
    "unica.dcs.compile",
    "unica.dcs.edit",
}
REMOVED_TOOLS = {name.replace(".dcs.", ".skd.") for name in EXPECTED_TOOLS}
EXPECTED_SKILLS = {
    "dcs-compile",
    "dcs-edit",
}
REMOVED_SKILLS = {name.replace("dcs-", "skd-") for name in EXPECTED_SKILLS}
SKD_IDENTIFIER = re.compile(r"(?<![A-Za-z0-9])(?:skd|Skd|SKD)")


class DcsNamingContractTests(unittest.TestCase):
    def test_public_dcs_migration_is_atomic_without_skd_aliases(self) -> None:
        registry = (
            REPO_ROOT / "crates" / "unica-coder" / "src" / "application" / "mod.rs"
        ).read_text(encoding="utf-8")
        domain_surface = set(
            re.findall(r'name: "(unica\.(?:dcs|skd)\.[^"]+)"', registry)
        )

        self.assertEqual(domain_surface, EXPECTED_TOOLS)
        self.assertTrue(REMOVED_TOOLS.isdisjoint(domain_surface))

    def test_prompt_visible_dcs_skills_replace_skd_skills(self) -> None:
        skill_root = REPO_ROOT / "plugins" / "unica" / "skills"
        skill_names = {path.name for path in skill_root.iterdir() if path.is_dir()}

        self.assertTrue(EXPECTED_SKILLS <= skill_names)
        self.assertTrue(REMOVED_SKILLS.isdisjoint(skill_names))
        for skill in EXPECTED_SKILLS:
            header = (skill_root / skill / "SKILL.md").read_text(encoding="utf-8")
            self.assertIn(f"name: {skill}", header)
            # Имя предмета осталось DCS, а маршрут стал каноническим: работу со
            # схемой компоновки ведёт `unica.apply`, а не снятый `unica.dcs.*`.
            self.assertIn("unica.apply", header)
            self.assertNotIn("unica.dcs.", header)


    def test_provenance_names_local_dcs_contract_but_preserves_donor_paths(self) -> None:
        path = REPO_ROOT / "docs" / "provenance" / "skill-upstreams.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        entries = {
            entry["skill"]: entry
            for upstream in data["upstreams"]
            for entry in upstream["entries"]
        }

        self.assertTrue(EXPECTED_SKILLS <= entries.keys())
        self.assertTrue(REMOVED_SKILLS.isdisjoint(entries.keys()))
        for skill in EXPECTED_SKILLS:
            entry = entries[skill]
            active_contract = json.dumps(
                {
                    "notes": entry.get("notes"),
                    "localPaths": entry.get("localPaths"),
                    "contractPaths": entry.get("contractPaths"),
                },
                ensure_ascii=False,
            )
            self.assertIsNone(SKD_IDENTIFIER.search(active_contract), skill)
            self.assertTrue(
                any("skd" in upstream_path.lower() for upstream_path in entry["upstreamPaths"]),
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
