from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
EXPECTED_TOOLS = {
    "unica.dcs.compile",
    "unica.dcs.edit",
    "unica.dcs.info",
    "unica.dcs.validate",
}
REMOVED_TOOLS = {name.replace(".dcs.", ".skd.") for name in EXPECTED_TOOLS}
EXPECTED_SKILLS = {
    "dcs-compile",
    "dcs-edit",
    "dcs-info",
    "dcs-validate",
}
REMOVED_SKILLS = {name.replace("dcs-", "skd-") for name in EXPECTED_SKILLS}
SKD_IDENTIFIER = re.compile(r"(?<![A-Za-z0-9])(?:skd|Skd|SKD)")
DSC_IDENTIFIER = re.compile(r"(?<![A-Za-z0-9])(?:dsc|Dsc|DSC)")
TEXT_SUFFIXES = {
    "",
    ".json",
    ".md",
    ".py",
    ".rs",
    ".sh",
    ".toml",
    ".xml",
    ".yaml",
    ".yml",
}


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
            self.assertIn(f"unica.dcs.{skill.removeprefix('dcs-')}", header)

    def test_active_english_identifiers_use_dcs_and_never_dsc(self) -> None:
        violations: list[str] = []
        for path in self.active_text_paths():
            relative = path.relative_to(REPO_ROOT).as_posix()
            if SKD_IDENTIFIER.search(relative):
                violations.append(f"{relative}: path contains SKD identifier")
            if DSC_IDENTIFIER.search(relative):
                violations.append(f"{relative}: path contains DSC identifier")

            text = self.text_for_naming_scan(path)
            for line_number, line in enumerate(text.splitlines(), start=1):
                if SKD_IDENTIFIER.search(line):
                    violations.append(f"{relative}:{line_number}: {line.strip()}")
                if DSC_IDENTIFIER.search(line):
                    violations.append(f"{relative}:{line_number}: {line.strip()}")

        self.assertEqual(violations, [])

    def test_provenance_names_local_dcs_contract_but_preserves_donor_paths(self) -> None:
        path = REPO_ROOT / "plugins" / "unica" / "provenance" / "skill-upstreams.json"
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

    def active_text_paths(self) -> list[Path]:
        roots = [
            REPO_ROOT / "README.md",
            REPO_ROOT / ".github",
            REPO_ROOT / "crates" / "unica-coder" / "src",
            REPO_ROOT / "plugins" / "unica",
            REPO_ROOT / "scripts",
            REPO_ROOT / "spec",
            REPO_ROOT / "tests" / "ci",
        ]
        excluded = {
            "plugins/unica/provenance/skill-upstreams.json",
            "spec/decisions/0011-canonical-dcs-domain.md",
            "tests/ci/test_dcs_naming_contract.py",
        }
        paths: list[Path] = []
        for root in roots:
            candidates = [root] if root.is_file() else root.rglob("*")
            for path in candidates:
                if not path.is_file() or path.suffix not in TEXT_SUFFIXES:
                    continue
                relative = path.relative_to(REPO_ROOT).as_posix()
                if relative in excluded:
                    continue
                if relative.startswith("plugins/unica/provenance/reviews/"):
                    continue
                if relative.startswith("spec/plans/"):
                    continue
                paths.append(path)
        return sorted(set(paths))

    def text_for_naming_scan(self, path: Path) -> str:
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(REPO_ROOT).as_posix()
        if relative == "plugins/unica/README.md":
            text = re.sub(
                r"\n## DCS naming migration\n.*?(?=\n## )",
                "",
                text,
                flags=re.DOTALL,
            )
        return text


if __name__ == "__main__":
    unittest.main()
