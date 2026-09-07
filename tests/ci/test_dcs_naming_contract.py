from __future__ import annotations

import json
import os
import re
import subprocess
import tempfile
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
ALLOWED_DONOR_SKD_LINES = {
    "scripts/ci/refresh-cc-1c-parity.py": {
        '"skd-compile": "dcs-compile",',
        '"skd-edit": "dcs-edit",',
        '"skd-info": "dcs-info",',
        '"skd-validate": "dcs-validate",',
    },
    "tests/ci/test_skill_provenance.py": {
        '"dcs-compile": ["tests/skills/cases/skd-compile/**"],',
    },
    "tests/ci/test_unica_mcp_script_parity.py": {
        '"skd-compile": "unica.dcs.compile",',
    },
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
        self.assertEqual(naming_violations(REPO_ROOT), [])

    def test_naming_scan_skips_what_git_ignores(self) -> None:
        """`.DS_Store` has no suffix, so the suffix filter alone admits the binary.

        Finder drops it into any directory; git ignores it, so the scan must
        too, while an extensionless tracked file and an unstaged text file
        are still read.
        """
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "arch").mkdir()
            (root / ".gitignore").write_text(".DS_Store\n", encoding="utf-8")
            (root / "README.md").write_text("DCS only\n", encoding="utf-8")
            (root / "arch" / "LICENSE").write_text("SKD in a licence\n", encoding="utf-8")
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            (root / "arch" / ".DS_Store").write_bytes(b"\x00\x00\x00\x01Bud1\xa8\xff")
            (root / "arch" / "unstaged.md").write_text("DSC typo\n", encoding="utf-8")

            self.assertEqual(
                naming_violations(root),
                ["arch/LICENSE:1: SKD in a licence", "arch/unstaged.md:1: DSC typo"],
            )

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



def repository_files(repo_root: Path, *pathspecs: str) -> list[Path]:
    """Files git tracks or would track under `pathspecs`; ignored files never enter.

    `Path.rglob` also returns what the desktop drops into a checkout: `.DS_Store`,
    editor swap files, a locally built binary. One such file breaks a text scan on
    a developer machine while CI, whose checkout carries none of them, stays
    green. A file git ignores is not part of the repository, so it is not part
    of a scan; an untracked file git would accept still is, exactly as with
    `rglob`. A tracked file deleted from the working tree is still listed, so
    callers keep their `is_file()` guard.
    """
    listed = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *pathspecs],
        cwd=repo_root,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    return sorted(repo_root / os.fsdecode(raw_path) for raw_path in listed if raw_path)


def active_text_paths(repo_root: Path) -> list[Path]:
    roots = [
        "README.md",
        ".github",
        "crates/unica-coder/src",
        "plugins/unica",
        "scripts",
        "arch",
        "tests/ci",
    ]
    excluded = {
        "docs/provenance/skill-upstreams.json",
        "tests/ci/test_dcs_naming_contract.py",
    }
    paths: list[Path] = []
    for path in repository_files(repo_root, *roots):
        if not path.is_file() or path.suffix not in TEXT_SUFFIXES:
            continue
        relative = path.relative_to(repo_root).as_posix()
        if relative in excluded:
            continue
        if relative.startswith("docs/provenance/reviews/"):
            continue
        if relative.startswith("docs/plans/"):
            continue
        paths.append(path)
    return sorted(set(paths))


def text_for_naming_scan(repo_root: Path, path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    relative = path.relative_to(repo_root).as_posix()
    if relative == "plugins/unica/README.md":
        text = re.sub(
            r"\n## DCS naming migration\n.*?(?=\n## )",
            "",
            text,
            flags=re.DOTALL,
        )
    return text


def naming_violations(repo_root: Path) -> list[str]:
    violations: list[str] = []
    for path in active_text_paths(repo_root):
        relative = path.relative_to(repo_root).as_posix()
        if SKD_IDENTIFIER.search(relative):
            violations.append(f"{relative}: path contains SKD identifier")
        if DSC_IDENTIFIER.search(relative):
            violations.append(f"{relative}: path contains DSC identifier")

        text = text_for_naming_scan(repo_root, path)
        for line_number, line in enumerate(text.splitlines(), start=1):
            if (
                SKD_IDENTIFIER.search(line)
                and line.strip() not in ALLOWED_DONOR_SKD_LINES.get(relative, set())
            ):
                violations.append(f"{relative}:{line_number}: {line.strip()}")
            if DSC_IDENTIFIER.search(line):
                violations.append(f"{relative}:{line_number}: {line.strip()}")
    return violations


if __name__ == "__main__":
    unittest.main()
