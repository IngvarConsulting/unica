from __future__ import annotations

import copy
import json
import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
HISTORICAL_SKILLS = {"dcs-compile", "dcs-edit"}
REMOVED_SKILLS = HISTORICAL_SKILLS | {
    "dcs-decompile", "skd-compile", "skd-edit", "skd-decompile"
}


def operations_with_checked_publication(scenarios: list[dict]) -> set[str]:
    """Registration guard only: the medium corpus executes these wires."""
    covered = set()
    for scenario in scenarios:
        tokens = {}
        for step in scenario["wire"]:
            if step["tool"] != "unica.apply" or step["expect"] != ["ok"]:
                continue
            args = step.get("args", {})
            assertions = step.get("assertions", [])
            if {"pointer": "/data/effects", "eq": 0} in assertions:
                continue
            if {"pointer": "/data/mode", "eq": "preview"} in assertions:
                for name, capture in step.get("captures", {}).items():
                    if capture.get("pointer") == "/data/executionToken":
                        tokens["$capture." + name] = {op["op"] for op in args.get("ops", [])}
            token = args.get("executionToken")
            if (
                token in tokens
                and {"pointer": "/data/mode", "eq": "published"} in assertions
                and any(
                    check.get("kind") == "xml" and "eq" in check
                    for check in step.get("fileAssertions", [])
                )
            ):
                covered.update(tokens[token])
    return covered


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

    def test_every_dcs_primitive_has_content_checked_publication(self) -> None:
        registry = (REPO_ROOT / "crates/unica-coder/src/domain/apply.rs").read_text(encoding="utf-8")
        registered = set(re.findall(r'\(\s*"([^"]+)"\s*,\s*Dcs\s*,', registry))
        # Owner-approved scope of #1310: deleting both implementation and wire
        # must not silently narrow the promised set.
        self.assertEqual(len(registered), 53)
        scenarios = json.loads(
            (REPO_ROOT / "tests/fixtures/acceptance/scenario-corpus.json").read_text(encoding="utf-8")
        )["scenarios"]
        self.assertEqual(registered - operations_with_checked_publication(scenarios), set())

    def test_publication_coverage_rejects_disconnected_or_unchecked_wires(self) -> None:
        wire = [
            {
                "tool": "unica.apply", "expect": ["ok"],
                "args": {"ops": [{"op": "query.patch"}]},
                "assertions": [{"pointer": "/data/mode", "eq": "preview"}],
                "captures": {"token": {"pointer": "/data/executionToken"}},
            },
            {
                "tool": "unica.apply", "expect": ["ok"],
                "args": {"executionToken": "$capture.token"},
                "assertions": [{"pointer": "/data/mode", "eq": "published"}],
                "fileAssertions": [{"kind": "xml", "select": "./query", "eq": ["Changed"]}],
            },
        ]
        self.assertEqual(operations_with_checked_publication([{"wire": wire}]), {"query.patch"})
        for defect in ("missing-execute", "wrong-token", "no-xml", "refused", "noop-preview", "noop-published"):
            with self.subTest(defect=defect):
                mutant = copy.deepcopy(wire)
                if defect == "missing-execute":
                    mutant.pop()
                elif defect == "wrong-token":
                    mutant[1]["args"]["executionToken"] = "$capture.other"
                elif defect == "no-xml":
                    mutant[1]["fileAssertions"] = []
                elif defect == "refused":
                    mutant[0]["expect"] = ["refused"]
                elif defect == "noop-preview":
                    mutant[0]["assertions"].append({"pointer": "/data/effects", "eq": 0})
                else:
                    # Real no-op replies stay published; effects, not mode,
                    # distinguish them from a meaningful mutation (S380).
                    mutant[1]["assertions"].append({"pointer": "/data/effects", "eq": 0})
                self.assertEqual(operations_with_checked_publication([{"wire": mutant}]), set())

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
