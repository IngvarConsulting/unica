from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from scripts.ci import donor_parity_contract as contract


COMMIT = "1" * 40


class DonorParityFixture:
    def __init__(self, root: Path) -> None:
        self.repo_root = root
        self.snapshot_root = (
            root / "tests" / "fixtures" / "unica_mcp_script_parity" / "cc-1c-skills"
        )
        self.case_root = self.snapshot_root / "cases" / "demo"
        self.skills_root = self.snapshot_root / "skills"
        self.case_root.mkdir(parents=True)
        (self.skills_root / "demo" / "scripts").mkdir(parents=True)
        (self.skills_root / "cf-init" / "scripts").mkdir(parents=True)
        (self.skills_root / "demo-validate" / "scripts").mkdir(parents=True)
        (self.case_root / "fixtures" / "on-support").mkdir(parents=True)
        (root / "spec").mkdir()
        self.reviews_root = (
            root / "plugins" / "unica" / "provenance" / "reviews"
        )
        self.reviews_root.mkdir(parents=True)

        self.skill_config = self.case_root / "_skill.json"
        self.case_file = self.case_root / "basic.json"
        self.script = self.skills_root / "demo" / "scripts" / "demo.py"
        self.setup_script = self.skills_root / "cf-init" / "scripts" / "cf-init.py"
        self.validator = (
            self.skills_root / "demo-validate" / "scripts" / "demo-validate.py"
        )
        self.fixture_file = self.case_root / "fixtures" / "on-support" / "Configuration.xml"
        self.evidence = root / "spec" / "donor-parity.md"

        self.skill_config.write_text(
            json.dumps(
                {
                    "script": "demo/scripts/demo",
                    "setup": "empty-config",
                    "postValidate": {
                        "script": "demo-validate/scripts/demo-validate"
                    },
                }
            ),
            encoding="utf-8",
        )
        self.case_file.write_text(
            json.dumps(
                {
                    "name": "basic",
                    "setup": "fixture:on-support",
                    "preRun": [{"script": "cf-init/scripts/cf-init"}],
                }
            ),
            encoding="utf-8",
        )
        self.script.write_text("print('donor')\n", encoding="utf-8")
        self.setup_script.write_text("print('setup')\n", encoding="utf-8")
        self.validator.write_text("print('validate')\n", encoding="utf-8")
        self.fixture_file.write_text("<Configuration/>\n", encoding="utf-8")
        self.evidence.write_text("# Evidence\n", encoding="utf-8")

    def manifest(self) -> dict:
        files = []
        for path in sorted(self.snapshot_root.rglob("*")):
            if path.is_file():
                relative = path.relative_to(self.snapshot_root).as_posix()
                files.append(
                    {
                        "scope": "demo",
                        "upstreamPath": self.upstream_path(relative),
                        "localPath": relative,
                        "acceptedCommit": COMMIT,
                        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                    }
                )
        digest = contract.case_content_digest(self.snapshot_root, "demo/basic")
        scopes = {
            "demo": {
                "ownerSkill": "demo",
                "caseScope": "demo",
                "acceptedCommit": COMMIT,
                "contentDigest": contract.scope_content_digest(files, "demo"),
            }
        }
        return {
            "schemaVersion": 1,
            "upstreamId": "cc-1c-skills",
            "repository": "https://example.invalid/donor.git",
            "trackingRef": "main",
            "scopes": scopes,
            "files": files,
            "cases": {"demo/basic": {"scope": "demo", "contentDigest": digest}},
        }

    def provenance(self) -> dict:
        return {
            "schemaVersion": 1,
            "upstreams": [
                {
                    "id": "cc-1c-skills",
                    "repository": "https://example.invalid/donor.git",
                    "trackingRef": "main",
                    "role": "operation-parity",
                    "baselineCommit": COMMIT,
                    "entries": [
                        {
                            "skill": "demo",
                            "baselineCommit": COMMIT,
                            "parityBaselineCommit": COMMIT,
                            "upstreamPaths": [
                                ".claude/skills/demo/**",
                                "tests/skills/cases/demo/**",
                            ],
                        }
                    ],
                }
            ],
        }

    def relation(self, kind: str = "compatible") -> dict:
        observation = reviewed_observation()
        return {
            "caseId": "demo/basic",
            "relation": kind,
            "contentDigest": contract.case_content_digest(
                self.snapshot_root, "demo/basic"
            ),
            "reason": "Reviewed behavior differs without losing the shared outcome.",
            "evidence": ["spec/donor-parity.md"],
            "observation": observation,
            "observationFingerprint": contract.observation_fingerprint(observation),
        }

    @staticmethod
    def upstream_path(local_path: str) -> str:
        if local_path.startswith("cases/"):
            return f"tests/skills/{local_path}"
        if local_path.startswith("skills/"):
            return f".claude/{local_path}"
        raise AssertionError(local_path)


def reviewed_observation() -> dict:
    return {
        "donorOk": True,
        "unicaOk": True,
        "mismatchKind": "stdout_mismatch_snapshot_equal",
        "donorStdoutSha256": "2" * 64,
        "unicaStdoutSha256": "3" * 64,
        "donorStderrSha256": "4" * 64,
        "unicaStderrSha256": "4" * 64,
        "donorWorkspaceSha256": "5" * 64,
        "unicaWorkspaceSha256": "5" * 64,
        "donorExpectedFiles": {"out.xml": True},
        "unicaExpectedFiles": {"out.xml": True},
    }


class DonorParityContractTests(unittest.TestCase):
    def test_case_execution_profile_binds_scope_specific_path_projection(
        self,
    ) -> None:
        cfe_profile = contract.case_execution_profile("cfe-borrow/catalog")
        other_profile = contract.case_execution_profile("meta-compile/catalog")

        self.assertEqual(
            cfe_profile["workspacePathProjection"],
            {"ext": "extension"},
        )
        self.assertNotIn("workspacePathProjection", other_profile)

    def fixture(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        return DonorParityFixture(Path(temporary.name))

    def test_baseline_accepts_complete_snapshot(self) -> None:
        fixture = self.fixture()

        errors = contract.validate_baseline(
            fixture.snapshot_root, fixture.manifest(), fixture.provenance()
        )

        self.assertEqual(errors, [])

    def test_baseline_detects_changed_snapshot_bytes(self) -> None:
        fixture = self.fixture()
        manifest = fixture.manifest()
        fixture.script.write_text("print('changed')\n", encoding="utf-8")

        errors = contract.validate_baseline(
            fixture.snapshot_root, manifest, fixture.provenance()
        )

        self.assertIn("sha256 mismatch", "\n".join(errors))

    def test_baseline_rejects_extra_file_and_provenance_commit_drift(self) -> None:
        fixture = self.fixture()
        manifest = fixture.manifest()
        (fixture.snapshot_root / "extra.txt").write_text("extra\n", encoding="utf-8")
        provenance = fixture.provenance()
        provenance["upstreams"][0]["entries"][0]["parityBaselineCommit"] = "9" * 40

        errors = contract.validate_baseline(
            fixture.snapshot_root, manifest, provenance
        )

        joined = "\n".join(errors)
        self.assertIn("unmanifested snapshot file", joined)
        self.assertIn("provenance commit mismatch", joined)

    def test_baseline_rejects_traversal_duplicate_and_symlink(self) -> None:
        fixture = self.fixture()
        manifest = fixture.manifest()
        duplicate = dict(manifest["files"][0])
        manifest["files"].append(duplicate)
        manifest["files"][0]["upstreamPath"] = "../escape.py"
        symlink = fixture.snapshot_root / "linked.py"
        symlink.symlink_to(fixture.script)

        errors = contract.validate_baseline(
            fixture.snapshot_root, manifest, fixture.provenance()
        )

        joined = "\n".join(errors)
        self.assertIn("unsafe repository-relative path", joined)
        self.assertIn("duplicate localPath", joined)
        self.assertIn("symlink", joined)

    def test_changed_case_script_fixture_or_setup_changes_content_digest(self) -> None:
        for attribute in ("case_file", "script", "fixture_file", "setup_script"):
            with self.subTest(attribute=attribute):
                fixture = self.fixture()
                before = contract.case_content_digest(
                    fixture.snapshot_root, "demo/basic"
                )
                path = getattr(fixture, attribute)
                if path.suffix == ".json":
                    value = json.loads(path.read_text(encoding="utf-8"))
                    value["changed"] = True
                    path.write_text(json.dumps(value), encoding="utf-8")
                else:
                    path.write_text(path.read_text(encoding="utf-8") + "changed\n")

                after = contract.case_content_digest(
                    fixture.snapshot_root, "demo/basic"
                )

                self.assertNotEqual(before, after)

    def test_relation_rejects_same_gap_kind_with_changed_fingerprint(self) -> None:
        fixture = self.fixture()
        relation = fixture.relation()
        observation = reviewed_observation()
        observation["unicaStdoutSha256"] = "8" * 64

        errors = contract.validate_relation_observation(
            relation=relation,
            content_digest=relation["contentDigest"],
            observation=observation,
        )

        self.assertIn("observation fingerprint changed", "\n".join(errors))

    def test_exact_relation_requires_an_exact_observation(self) -> None:
        fixture = self.fixture()
        relation = fixture.relation("exact")
        relation.pop("observation")
        relation.pop("observationFingerprint")
        relation.pop("reason")
        relation.pop("evidence")

        errors = contract.validate_relation_observation(
            relation=relation,
            content_digest=relation["contentDigest"],
            observation=reviewed_observation(),
        )

        self.assertIn("exact relation observed a difference", "\n".join(errors))

    def test_relations_require_every_case_and_reject_extras(self) -> None:
        fixture = self.fixture()
        registry = {
            "schemaVersion": 1,
            "upstreamId": "cc-1c-skills",
            "relations": {"other/case": fixture.relation()},
        }

        errors = contract.validate_relations(
            fixture.repo_root, fixture.snapshot_root, registry
        )

        joined = "\n".join(errors)
        self.assertIn("missing relation for demo/basic", joined)
        self.assertIn("relation for unknown case other/case", joined)

    def test_non_exact_relation_requires_existing_safe_evidence(self) -> None:
        fixture = self.fixture()
        relation = fixture.relation("platform_override")
        relation["evidence"] = ["../outside.md", "spec/missing.md"]
        registry = {
            "schemaVersion": 1,
            "upstreamId": "cc-1c-skills",
            "relations": {"demo/basic": relation},
        }

        errors = contract.validate_relations(
            fixture.repo_root, fixture.snapshot_root, registry
        )

        joined = "\n".join(errors)
        self.assertIn("unsafe repository-relative path", joined)
        self.assertIn("evidence path does not exist", joined)

    def test_donor_ahead_requires_owner_and_decision(self) -> None:
        fixture = self.fixture()
        relation = fixture.relation("donor_ahead")
        registry = {
            "schemaVersion": 1,
            "upstreamId": "cc-1c-skills",
            "relations": {"demo/basic": relation},
        }

        errors = contract.validate_relations(
            fixture.repo_root, fixture.snapshot_root, registry
        )

        joined = "\n".join(errors)
        self.assertIn("owner is required", joined)
        self.assertIn("decision must be adopt or defer", joined)

    def test_reviewed_non_exact_relation_is_valid(self) -> None:
        fixture = self.fixture()
        registry = {
            "schemaVersion": 1,
            "upstreamId": "cc-1c-skills",
            "relations": {"demo/basic": fixture.relation()},
        }

        errors = contract.validate_relations(
            fixture.repo_root, fixture.snapshot_root, registry
        )

        self.assertEqual(errors, [])

    def test_baseline_scope_requires_matching_applied_refresh_review(self) -> None:
        fixture = self.fixture()
        manifest = fixture.manifest()
        manifest["scopes"]["demo"]["reviewId"] = "demo-refresh"
        review_path = fixture.reviews_root / "demo-refresh.json"
        review = {
            "reviewStatus": "reviewed",
            "applied": True,
            "targetCommit": COMMIT,
            "affectedSkills": ["demo"],
        }
        review_path.write_text(json.dumps(review), encoding="utf-8")

        self.assertEqual(
            contract.validate_refresh_reviews(fixture.repo_root, manifest),
            [],
        )

        review["applied"] = False
        review_path.write_text(json.dumps(review), encoding="utf-8")
        errors = contract.validate_refresh_reviews(
            fixture.repo_root, manifest
        )
        self.assertIn("not reviewed and applied", "\n".join(errors))


if __name__ == "__main__":
    unittest.main()
