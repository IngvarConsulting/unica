from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scripts.ci import donor_parity_contract as contract


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ci" / "refresh-cc-1c-parity.py"


def load_refresh_module():
    sys.modules.setdefault("donor_parity_contract", contract)
    spec = importlib.util.spec_from_file_location("refresh_cc_1c_parity", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class RefreshFixture:
    def __init__(
        self,
        root: Path,
        *,
        owner: str = "cfe-borrow",
        case_scope: str = "cfe-borrow",
        donor_skill: str | None = None,
        baseline_scope: str | None = None,
        validation_owner: str | None = None,
        validation_donor_skill: str | None = None,
    ) -> None:
        self.repo_root = root / "unica"
        self.upstream = self.repo_root / ".build" / "skill-upstreams" / "cc-1c-skills"
        self.snapshot = (
            self.repo_root
            / "tests"
            / "fixtures"
            / "unica_mcp_script_parity"
            / "cc-1c-skills"
        )
        self.fixtures_root = self.snapshot.parent
        self.provenance_path = (
            self.repo_root / "docs" / "arch-v1" / "provenance" / "skill-upstreams.json"
        )
        self.reviews_root = self.repo_root / "docs" / "provenance" / "reviews"
        self.owner = owner
        self.case_scope = case_scope
        self.donor_skill = donor_skill or owner
        self.baseline_scope = baseline_scope or owner
        self.validation_owner = validation_owner
        self.validation_donor_skill = validation_donor_skill
        self.case_id = f"{case_scope}/basic"

        self.upstream.mkdir(parents=True)
        self.provenance_path.parent.mkdir(parents=True)
        self.reviews_root.mkdir(parents=True)
        self.fixtures_root.mkdir(parents=True)
        self._git("init", "-b", "main")
        self._git("config", "user.email", "ci@example.invalid")
        self._git("config", "user.name", "CI")
        self._write_upstream_case("print('v1')\n")
        self.initial_commit = self.commit("initial")
        self._write_accepted_snapshot(self.initial_commit)

    def _git(self, *arguments: str) -> str:
        result = subprocess.run(
            ["git", *arguments],
            cwd=self.upstream,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=True,
        )
        return result.stdout.strip()

    def _write_upstream_case(self, script_text: str) -> None:
        cases = self.upstream / "tests" / "skills" / "cases" / self.case_scope
        scripts = self.upstream / ".claude" / "skills" / self.donor_skill / "scripts"
        cases.mkdir(parents=True, exist_ok=True)
        scripts.mkdir(parents=True, exist_ok=True)
        config = {
            "script": f"{self.donor_skill}/scripts/{self.donor_skill}",
            "setup": "none",
            "args": [],
        }
        if self.validation_donor_skill is not None:
            config["postValidate"] = {
                "script": (
                    f"{self.validation_donor_skill}/scripts/"
                    f"{self.validation_donor_skill}"
                )
            }
        (cases / "_skill.json").write_text(
            json.dumps(config),
            encoding="utf-8",
        )
        (cases / "basic.json").write_text(
            json.dumps({"name": "basic"}),
            encoding="utf-8",
        )
        (scripts / f"{self.donor_skill}.py").write_text(script_text, encoding="utf-8")
        if self.validation_donor_skill is not None:
            validation_scripts = (
                self.upstream
                / ".claude"
                / "skills"
                / self.validation_donor_skill
                / "scripts"
            )
            validation_scripts.mkdir(parents=True, exist_ok=True)
            (validation_scripts / f"{self.validation_donor_skill}.py").write_text(
                "print('validate')\n",
                encoding="utf-8",
            )

    def commit(self, message: str) -> str:
        self._git("add", "-A")
        self._git("-c", "commit.gpgsign=false", "commit", "-m", message)
        return self._git("rev-parse", "HEAD")

    def change_unrelated(self) -> str:
        readme = self.upstream / "README.md"
        previous = readme.read_text(encoding="utf-8") if readme.is_file() else ""
        readme.write_text(previous + "changed\n", encoding="utf-8")
        return self.commit("unrelated")

    def change_script(self) -> str:
        script = (
            self.upstream
            / ".claude"
            / "skills"
            / self.donor_skill
            / "scripts"
            / f"{self.donor_skill}.py"
        )
        script.write_text("print('v2')\n", encoding="utf-8")
        return self.commit("script")

    def remove_case(self) -> str:
        (self.upstream / "tests" / "skills" / "cases" / self.case_scope / "basic.json").unlink()
        return self.commit("remove case")

    def add_script_symlink(self) -> str:
        scripts = self.upstream / ".claude" / "skills" / self.donor_skill / "scripts"
        (scripts / "linked.py").symlink_to(f"{self.donor_skill}.py")
        return self.commit("symlink")

    def _write_accepted_snapshot(self, commit: str) -> None:
        case_source = self.upstream / "tests" / "skills" / "cases" / self.case_scope
        script_source = self.upstream / ".claude" / "skills" / self.donor_skill / "scripts"
        shutil.copytree(case_source, self.snapshot / "cases" / self.case_scope)
        shutil.copytree(script_source, self.snapshot / "skills" / self.donor_skill / "scripts")
        if self.validation_donor_skill is not None:
            validation_source = (
                self.upstream
                / ".claude"
                / "skills"
                / self.validation_donor_skill
                / "scripts"
            )
            shutil.copytree(
                validation_source,
                self.snapshot / "skills" / self.validation_donor_skill / "scripts",
            )
        provenance = self.provenance(commit)
        self.provenance_path.write_text(
            json.dumps(provenance, indent=2) + "\n",
            encoding="utf-8",
        )
        baseline = self.baseline(commit)
        (self.fixtures_root / "donor-baseline.json").write_text(
            json.dumps(baseline, indent=2) + "\n",
            encoding="utf-8",
        )
        relation = {
            "caseId": self.case_id,
            "relation": "exact",
            "contentDigest": contract.case_content_digest(
                self.snapshot, self.case_id
            ),
        }
        (self.fixtures_root / "donor-relations.json").write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "upstreamId": "cc-1c-skills",
                    "relations": {self.case_id: relation},
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    def provenance(self, commit: str) -> dict:
        entries = [
            {
                "skill": self.owner,
                "baselineCommit": commit,
                "parityBaselineCommit": commit,
                "upstreamPaths": [
                    f".claude/skills/{self.donor_skill}/scripts/**",
                    f"tests/skills/cases/{self.case_scope}/**",
                ],
            }
        ]
        if self.validation_donor_skill is not None:
            entries.append(
                {
                    "skill": self.validation_owner,
                    "baselineCommit": commit,
                    "parityBaselineCommit": commit,
                    "upstreamPaths": [
                        f".claude/skills/{self.validation_donor_skill}/scripts/**"
                    ],
                }
            )
        return {
            "schemaVersion": 1,
            "upstreams": [
                {
                    "id": "cc-1c-skills",
                    "repository": "https://example.invalid/donor.git",
                    "trackingRef": "main",
                    "role": "operation-parity",
                    "baselineCommit": commit,
                    "entries": entries,
                }
            ],
        }

    def baseline(self, commit: str) -> dict:
        files = []
        for path in sorted(self.snapshot.rglob("*")):
            if not path.is_file():
                continue
            local_path = path.relative_to(self.snapshot).as_posix()
            if local_path.startswith("cases/"):
                upstream_path = f"tests/skills/{local_path}"
                scope = self.baseline_scope
            else:
                upstream_path = f".claude/{local_path}"
                donor_skill = Path(local_path).parts[1]
                scope = (
                    self.validation_donor_skill
                    if donor_skill == self.validation_donor_skill
                    else self.baseline_scope
                )
            files.append(
                {
                    "scope": scope,
                    "upstreamPath": upstream_path,
                    "localPath": local_path,
                    "acceptedCommit": commit,
                    "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                }
            )
        scopes = {
            self.baseline_scope: {
                "ownerSkill": self.owner,
                "caseScopes": [self.case_scope],
                "acceptedCommit": commit,
                "contentDigest": contract.scope_content_digest(
                    files, self.baseline_scope
                ),
            }
        }
        if self.validation_donor_skill is not None:
            scopes[self.validation_donor_skill] = {
                "ownerSkill": self.validation_owner,
                "caseScopes": [],
                "acceptedCommit": commit,
                "contentDigest": contract.scope_content_digest(
                    files, self.validation_donor_skill
                ),
            }
        return {
            "schemaVersion": 1,
            "upstreamId": "cc-1c-skills",
            "repository": "https://example.invalid/donor.git",
            "trackingRef": "main",
            "scopes": scopes,
            "files": files,
            "cases": {
                self.case_id: {
                    "scope": self.baseline_scope,
                    "contentDigest": contract.case_content_digest(
                        self.snapshot, self.case_id
                    ),
                }
            },
        }

    def prepare(self, target: str, *extra: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "prepare",
                "--repo-root",
                str(self.repo_root),
                "--upstream-cache",
                str(self.upstream),
                "--target",
                target,
                "--review-id",
                "test-refresh",
                *extra,
            ],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    @property
    def review_path(self) -> Path:
        return (
            self.repo_root
            / ".build"
            / "donor-parity-refresh"
            / "test-refresh"
            / "review.json"
        )

    def load_review(self) -> dict:
        return json.loads(self.review_path.read_text(encoding="utf-8"))

    def write_review(self, review: dict) -> None:
        self.review_path.write_text(
            json.dumps(review, indent=2) + "\n",
            encoding="utf-8",
        )

    def apply(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "apply",
                "--repo-root",
                str(self.repo_root),
                "--review",
                str(self.review_path),
            ],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def snapshot_digest(self) -> str:
        return contract.sha256_json(
            [
                (
                    path.relative_to(self.snapshot).as_posix(),
                    hashlib.sha256(path.read_bytes()).hexdigest(),
                )
                for path in sorted(self.snapshot.rglob("*"))
                if path.is_file()
            ]
        )


class RefreshCc1cParityTests(unittest.TestCase):
    def fixture(self, **kwargs) -> RefreshFixture:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        return RefreshFixture(Path(temporary.name), **kwargs)

    def test_build_baseline_preserves_renamed_owner_for_historical_scope(self) -> None:
        module = load_refresh_module()
        with tempfile.TemporaryDirectory() as temporary:
            snapshot = Path(temporary) / "snapshot"
            script = snapshot / "skills" / "meta-compile" / "scripts" / "meta-compile.py"
            script.parent.mkdir(parents=True)
            script.write_text("print('historical')\n", encoding="utf-8")
            accepted = "1" * 40
            old_baseline = {
                "scopes": {
                    "meta-compile": {
                        "ownerSkill": "meta-add",
                        "acceptedCommit": accepted,
                        "reviewId": "historical-review",
                    }
                }
            }

            baseline = module.build_baseline(
                snapshot,
                {
                    "repository": "https://example.invalid/donor.git",
                    "trackingRef": "main",
                },
                target_commit="2" * 40,
                affected_skills=set(),
                old_baseline=old_baseline,
                review_id="new-review",
            )

        self.assertEqual(
            baseline["scopes"]["meta-compile"]["ownerSkill"],
            "meta-add",
        )

    def test_prepare_and_apply_select_renamed_meta_owner_without_rewriting_donor_scopes(self) -> None:
        fixture = self.fixture(
            owner="meta-add",
            case_scope="meta-compile",
            donor_skill="meta-compile",
            baseline_scope="meta-compile",
            validation_owner="meta-info",
            validation_donor_skill="meta-validate",
        )
        target = fixture.change_unrelated()

        prepared = fixture.prepare(target, "--skill", "meta-add")

        self.assertEqual(prepared.returncode, 0, prepared.stderr)
        review = fixture.load_review()
        self.assertEqual(review["selectedSkills"], ["meta-add"])
        self.assertEqual(review["selectedCaseScopes"], ["meta-compile"])
        self.assertEqual(review["affectedSkills"], ["meta-add", "meta-info"])
        candidate_root = fixture.repo_root / review["candidatePath"]
        baseline = contract.load_json(candidate_root / "donor-baseline.json")
        self.assertEqual(
            baseline["scopes"]["meta-compile"]["ownerSkill"],
            "meta-add",
        )
        self.assertEqual(
            baseline["scopes"]["meta-validate"]["ownerSkill"],
            "meta-info",
        )
        self.assertEqual(
            baseline["scopes"]["meta-compile"]["acceptedCommit"],
            target,
        )
        self.assertEqual(
            baseline["scopes"]["meta-validate"]["acceptedCommit"],
            target,
        )
        self.assertEqual(
            baseline["cases"][fixture.case_id]["scope"],
            "meta-compile",
        )

        review["reviewStatus"] = "reviewed"
        fixture.write_review(review)
        applied = fixture.apply()

        self.assertEqual(applied.returncode, 0, applied.stderr)
        accepted = contract.load_json(
            fixture.fixtures_root / "donor-baseline.json"
        )
        self.assertEqual(accepted["scopes"], baseline["scopes"])

    def test_prepare_carries_only_unchanged_content_relations(self) -> None:
        fixture = self.fixture()
        target = fixture.change_unrelated()

        result = fixture.prepare(target)

        self.assertEqual(result.returncode, 0, result.stderr)
        review = fixture.load_review()
        self.assertEqual(review["carriedRelations"], [fixture.case_id])
        self.assertEqual(review["needsReview"], [])

    def test_prepare_invalidates_relation_when_script_changes(self) -> None:
        fixture = self.fixture()
        target = fixture.change_script()

        result = fixture.prepare(target)

        self.assertEqual(result.returncode, 0, result.stderr)
        review = fixture.load_review()
        self.assertEqual(review["carriedRelations"], [])
        self.assertEqual(review["needsReview"], [fixture.case_id])
        self.assertEqual(review["changedCases"], [fixture.case_id])

    def test_prepare_is_dry_and_rejects_unknown_selected_skill(self) -> None:
        fixture = self.fixture()
        before = fixture.snapshot_digest()
        target = fixture.change_unrelated()

        result = fixture.prepare(target, "--skill", "unknown")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unknown selected skill", result.stderr)
        self.assertEqual(fixture.snapshot_digest(), before)

    def test_prepare_rejects_symlink_in_selected_scripts(self) -> None:
        fixture = self.fixture()
        target = fixture.add_script_symlink()

        result = fixture.prepare(target)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("symlink", result.stderr)

    def test_removed_case_requires_explicit_review(self) -> None:
        fixture = self.fixture()
        target = fixture.remove_case()

        result = fixture.prepare(target)

        self.assertEqual(result.returncode, 0, result.stderr)
        review = fixture.load_review()
        self.assertEqual(review["removedCases"], [fixture.case_id])
        self.assertEqual(review["needsReview"], [fixture.case_id])

    def test_apply_rejects_unresolved_review(self) -> None:
        fixture = self.fixture()
        target = fixture.change_script()
        self.assertEqual(fixture.prepare(target).returncode, 0)

        result = fixture.apply()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unresolved donor relations", result.stderr)

    def test_apply_rejects_target_that_moved_after_review(self) -> None:
        fixture = self.fixture()
        target = fixture.change_unrelated()
        self.assertEqual(fixture.prepare("main").returncode, 0)
        review = fixture.load_review()
        review["reviewStatus"] = "reviewed"
        fixture.write_review(review)
        fixture.change_unrelated()

        result = fixture.apply()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("target ref moved", result.stderr)
        self.assertNotEqual(target, fixture._git("rev-parse", "main"))

    def test_apply_updates_snapshot_manifest_provenance_relations_and_review(self) -> None:
        fixture = self.fixture()
        target = fixture.change_script()
        before = fixture.snapshot_digest()
        self.assertEqual(fixture.prepare(target).returncode, 0)
        review = fixture.load_review()
        candidate_snapshot = (
            fixture.repo_root / review["candidatePath"] / "cc-1c-skills"
        )
        relation = {
            "caseId": fixture.case_id,
            "relation": "exact",
            "contentDigest": contract.case_content_digest(
                candidate_snapshot, fixture.case_id
            ),
        }
        review["caseDecisions"][fixture.case_id] = {
            "status": "reviewed",
            "relation": relation,
        }
        review["reviewStatus"] = "reviewed"
        fixture.write_review(review)

        result = fixture.apply()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotEqual(fixture.snapshot_digest(), before)
        baseline = contract.load_json(
            fixture.fixtures_root / "donor-baseline.json"
        )
        provenance = contract.load_json(fixture.provenance_path)
        relations = contract.load_json(
            fixture.fixtures_root / "donor-relations.json"
        )
        self.assertEqual(
            contract.validate_baseline(fixture.snapshot, baseline, provenance),
            [],
        )
        self.assertEqual(
            contract.validate_relations(
                fixture.repo_root, fixture.snapshot, relations
            ),
            [],
        )
        self.assertEqual(
            baseline["scopes"][fixture.owner]["acceptedCommit"], target
        )
        entry = provenance["upstreams"][0]["entries"][0]
        self.assertEqual(entry["baselineCommit"], fixture.initial_commit)
        self.assertEqual(entry["parityBaselineCommit"], target)
        tracked_review = fixture.reviews_root / "test-refresh.json"
        self.assertTrue(tracked_review.is_file())
        self.assertEqual(
            json.loads(tracked_review.read_text(encoding="utf-8"))["targetCommit"],
            target,
        )

    def test_form_compile_from_object_scope_is_owned_by_form_compile(self) -> None:
        fixture = self.fixture(
            owner="form-compile",
            case_scope="form-compile-from-object",
        )
        target = fixture.change_unrelated()

        result = fixture.prepare(target, "--skill", "form-compile")

        self.assertEqual(result.returncode, 0, result.stderr)
        review = fixture.load_review()
        self.assertEqual(review["selectedSkills"], ["form-compile"])
        self.assertEqual(
            review["selectedCaseScopes"], ["form-compile-from-object"]
        )


if __name__ == "__main__":
    unittest.main()
