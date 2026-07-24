from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ci" / "generate-donor-skill-matrix.py"
COMMIT = "a" * 40


class MatrixFixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.baseline_path = (
            root
            / "tests"
            / "fixtures"
            / "unica_mcp_script_parity"
            / "donor-baseline.json"
        )
        self.relations_path = self.baseline_path.with_name("donor-relations.json")
        self.provenance_path = (
            root / "plugins" / "unica" / "provenance" / "skill-upstreams.json"
        )
        self.mapping_path = (
            root / "plugins" / "unica" / "provenance" / "donor-skill-map.json"
        )
        self.output_path = root / "docs" / "donor-skill-parity.md"
        (root / "plugins" / "unica" / "skills" / "local-a").mkdir(
            parents=True
        )
        application = root / "crates" / "unica-coder" / "src" / "application"
        application.mkdir(parents=True)
        (application / "mod.rs").write_text(
            'ToolSpec { name: "unica.local.a" }\n', encoding="utf-8"
        )
        self.write_contracts()

    def write_contracts(self) -> None:
        self.baseline_path.parent.mkdir(parents=True, exist_ok=True)
        self.baseline_path.write_text(
            json.dumps(
                {
                    "schemaVersion": 2,
                    "upstreamId": "cc-1c-skills",
                    "repository": "https://example.invalid/cc-1c-skills.git",
                    "trackingRef": "main",
                    "acceptedCommit": COMMIT,
                    "corpusSkills": {
                        "donor-a": {
                            "scripts": [
                                "skills/donor-a/scripts/donor-a.py"
                            ]
                        },
                        "donor-b": {"scripts": []},
                    },
                    "corpusTests": {
                        "caseScopes": {
                            "donor-a": {"caseIds": ["donor-a/basic"]},
                            "donor-b": {"caseIds": ["donor-b/future"]},
                        },
                        "sharedFiles": ["case-runner/runner.js"],
                        "suites": {
                            "web-test": {
                                "files": ["suites/web-test/smoke.test.js"]
                            }
                        },
                    },
                    "executableCaseScopes": ["donor-a"],
                    "scopes": {
                        "local-a": {
                            "ownerSkill": "local-a",
                            "caseScopes": ["donor-a"],
                            "acceptedCommit": COMMIT,
                            "reviewId": "review",
                            "contentDigest": "b" * 64,
                        }
                    },
                    "files": [],
                    "cases": {
                        "donor-a/basic": {
                            "scope": "local-a",
                            "digestKind": "execution",
                            "contentDigest": "c" * 64,
                        },
                        "donor-b/future": {
                            "scope": None,
                            "digestKind": "corpus",
                            "contentDigest": "d" * 64,
                        },
                    },
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        self.relations_path.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "upstreamId": "cc-1c-skills",
                    "relations": {
                        "donor-a/basic": {
                            "caseId": "donor-a/basic",
                            "relation": "exact",
                            "contentDigest": "c" * 64,
                        }
                    },
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        self.provenance_path.parent.mkdir(parents=True, exist_ok=True)
        self.provenance_path.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "upstreams": [
                        {
                            "id": "cc-1c-skills",
                            "entries": [
                                {
                                    "skill": "donor-a",
                                    "status": "ported-to-unica",
                                    "localPaths": [
                                        "plugins/unica/skills/local-a"
                                    ],
                                }
                            ],
                        }
                    ],
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        self.mapping_path.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "upstreamId": "cc-1c-skills",
                    "skills": {
                        "donor-a": {
                            "unicaSkills": ["local-a"],
                            "tools": [
                                {
                                    "name": "unica.local.a",
                                    "relation": "direct",
                                }
                            ],
                            "caseScopes": ["donor-a"],
                            "testSuites": [],
                        },
                        "donor-b": {
                            "unicaSkills": [],
                            "tools": [],
                            "caseScopes": ["donor-b"],
                            "testSuites": ["web-test"],
                        },
                    },
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    def run(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--repo-root",
                str(self.root),
                *arguments,
            ],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )


class GenerateDonorSkillMatrixTests(unittest.TestCase):
    def fixture(self) -> MatrixFixture:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        return MatrixFixture(Path(temporary.name))

    def test_write_generates_requested_columns_and_check_detects_drift(self) -> None:
        fixture = self.fixture()

        result = fixture.run("--write")

        self.assertEqual(result.returncode, 0, result.stderr)
        document = fixture.output_path.read_text(encoding="utf-8")
        for heading in (
            "Какие скиллы есть у Николая",
            "Заимствовали ли мы этот скилл в Unica",
            "Какие наши тулзы есть для этого скилла",
            "Какие скрипты Николая используются у него в скиле",
            "Какое состояние парити для этого скилла",
            "Какое состояние тестового корпуса для этого скилла",
        ):
            self.assertIn(heading, document)
        self.assertIn("Да — `local-a`", document)
        self.assertIn("`unica.local.a` (direct)", document)
        self.assertIn("`skills/donor-a/scripts/donor-a.py`", document)
        self.assertIn("exact: 1", document)
        self.assertIn("1/1 JSON", document)
        self.assertIn("web-test: 1 files", document)

        self.assertEqual(fixture.run("--check").returncode, 0)
        fixture.output_path.write_text("stale\n", encoding="utf-8")
        stale = fixture.run("--check")
        self.assertNotEqual(stale.returncode, 0)
        self.assertIn("out of date", stale.stderr)

    def test_rejects_unknown_tool_and_missing_donor_mapping(self) -> None:
        fixture = self.fixture()
        mapping = json.loads(fixture.mapping_path.read_text(encoding="utf-8"))
        mapping["skills"].pop("donor-b")
        mapping["skills"]["donor-a"]["tools"][0]["name"] = "unica.missing"
        fixture.mapping_path.write_text(
            json.dumps(mapping, indent=2) + "\n", encoding="utf-8"
        )

        result = fixture.run("--write")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("mapping is missing donor skill: donor-b", result.stderr)
        self.assertIn("unknown Unica tool: unica.missing", result.stderr)


if __name__ == "__main__":
    unittest.main()
