from __future__ import annotations

import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path


def load_upstream_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "check-skill-upstreams.py"
    spec = importlib.util.spec_from_file_location("check_skill_upstreams", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SkillProvenanceTests(unittest.TestCase):
    def repo_root(self) -> Path:
        return Path(__file__).resolve().parents[2]

    def provenance_path(self) -> Path:
        return self.repo_root() / "plugins" / "unica" / "provenance" / "skill-upstreams.json"

    def reviews_dir(self) -> Path:
        return self.repo_root() / "plugins" / "unica" / "provenance" / "reviews"

    def upstream_review_path(self) -> Path:
        return self.reviews_dir() / "2026-06-15-upstream-review.json"

    def product_backlog_path(self) -> Path:
        return self.reviews_dir() / "2026-06-18-product-update-backlog.json"

    def load_provenance(self) -> dict:
        return json.loads(self.provenance_path().read_text(encoding="utf-8"))

    def load_upstream_review(self) -> dict:
        return json.loads(self.upstream_review_path().read_text(encoding="utf-8"))

    def load_product_backlog(self) -> dict:
        return json.loads(self.product_backlog_path().read_text(encoding="utf-8"))

    def test_provenance_index_validates_offline(self) -> None:
        module = load_upstream_module()

        report = module.validate_index(self.repo_root(), self.provenance_path())

        self.assertEqual(report.errors, [])

    def test_tracking_ref_resolution_prefers_fetched_remote_branch(self) -> None:
        module = load_upstream_module()

        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.PIPE)
            subprocess.run(["git", "config", "user.email", "ci@example.invalid"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "CI"], cwd=repo, check=True)

            marker = repo / "marker.txt"
            marker.write_text("stale\n", encoding="utf-8")
            subprocess.run(["git", "add", "marker.txt"], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "stale"], cwd=repo, check=True, stdout=subprocess.PIPE)
            stale_commit = module.git_output(["rev-parse", "HEAD"], cwd=repo)

            marker.write_text("fresh\n", encoding="utf-8")
            subprocess.run(["git", "commit", "-am", "fresh"], cwd=repo, check=True, stdout=subprocess.PIPE)
            fresh_commit = module.git_output(["rev-parse", "HEAD"], cwd=repo)
            subprocess.run(["git", "update-ref", "refs/remotes/origin/main", fresh_commit], cwd=repo, check=True)
            subprocess.run(["git", "reset", "--hard", stale_commit], cwd=repo, check=True, stdout=subprocess.PIPE)

            self.assertEqual(module.git_output(["rev-parse", "main"], cwd=repo), stale_commit)
            self.assertEqual(module.git_output(["rev-parse", "origin/main"], cwd=repo), fresh_commit)
            self.assertEqual(module.resolve_ref(repo, "main"), fresh_commit)

    def test_provenance_index_lives_in_packaged_non_prompt_visible_area(self) -> None:
        path = self.provenance_path()

        self.assertTrue(path.is_file())
        self.assertIn("plugins/unica/provenance", path.as_posix())
        self.assertNotIn("plugins/unica/skills", path.as_posix())
        self.assertNotIn("plugins/unica/references", path.as_posix())

    def test_required_upstreams_are_present(self) -> None:
        data = self.load_provenance()
        upstreams = {item["id"]: item for item in data["upstreams"]}

        self.assertIn("cc-1c-skills", upstreams)
        self.assertIn("ai-rules-1c", upstreams)
        self.assertIn("v8-runner-rust", upstreams)
        self.assertEqual(upstreams["cc-1c-skills"]["role"], "operation-parity")
        self.assertEqual(upstreams["ai-rules-1c"]["role"], "guidance")
        self.assertEqual(upstreams["v8-runner-rust"]["role"], "runtime-tool-contract")
        self.assertEqual(upstreams["v8-runner-rust"]["toolLockRef"], "v8-runner")
        self.assertNotIn("baselineCommit", upstreams["v8-runner-rust"])

    def test_historical_donor_baselines_track_last_local_adaptation_not_current_head(self) -> None:
        data = self.load_provenance()
        upstreams = {item["id"]: item for item in data["upstreams"]}

        self.assertEqual(
            upstreams["cc-1c-skills"]["baselineCommit"],
            "f3466e19fdc37954c030e48daabcc192f0098fe7",
        )
        self.assertEqual(
            upstreams["cc-1c-skills"]["lastAdaptedLocalCommit"],
            "795505f2243cf3c93a95918467f99135af758e1b",
        )
        self.assertEqual(
            upstreams["ai-rules-1c"]["baselineCommit"],
            "484e550043a4cb749d59d0671329f3112e3ae668",
        )
        self.assertEqual(
            upstreams["ai-rules-1c"]["lastAdaptedLocalCommit"],
            "e5b4eeab4dac92e0c9f60d3f886aa2bb7ef79f80",
        )

    def test_tool_lock_ref_uses_tools_lock_as_single_binary_baseline(self) -> None:
        data = self.load_provenance()
        tool_lock = json.loads(
            (self.repo_root() / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        locked_tools = {tool["name"]: tool for tool in tool_lock["tools"]}

        runtime_source = next(item for item in data["upstreams"] if item["id"] == "v8-runner-rust")

        self.assertEqual(runtime_source["toolLockRef"], "v8-runner")
        self.assertIn(runtime_source["toolLockRef"], locked_tools)
        self.assertEqual(locked_tools["v8-runner"]["sourceTag"], "v0.5.1")
        self.assertEqual(locked_tools["v8-runner"]["sourceCommit"], "ad72f64222ab0a7e6dfd391adb437a956c0a2428")

    def test_rlm_tools_are_locked_to_reviewed_1_24_0_pair(self) -> None:
        tool_lock = json.loads(
            (self.repo_root() / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        locked_tools = {tool["name"]: tool for tool in tool_lock["tools"]}

        for name in ("rlm-tools-bsl", "rlm-bsl-index"):
            self.assertEqual(locked_tools[name]["version"], "1.24.0")
            self.assertEqual(locked_tools[name]["sourceTag"], "v1.24.0")
            self.assertEqual(
                locked_tools[name]["sourceCommit"],
                "28695871516319a8678f397244cb9ce3b20abfdb",
            )

    def test_bsl_analyzer_is_locked_to_reviewed_0_2_37(self) -> None:
        tool_lock = json.loads(
            (self.repo_root() / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        locked_tools = {tool["name"]: tool for tool in tool_lock["tools"]}

        self.assertEqual(locked_tools["bsl-analyzer"]["version"], "0.2.37")
        self.assertEqual(locked_tools["bsl-analyzer"]["sourceTag"], "v0.2.37")
        self.assertEqual(
            locked_tools["bsl-analyzer"]["sourceCommit"],
            "a59fb3e2cc11e822723e2e42257f64d92267c084",
        )

    def test_all_local_and_contract_paths_exist(self) -> None:
        data = self.load_provenance()
        missing = []
        for upstream in data["upstreams"]:
            for entry in upstream["entries"]:
                for key in ("localPaths", "contractPaths"):
                    for rel_path in entry.get(key, []):
                        if not (self.repo_root() / rel_path).exists():
                            missing.append(f"{upstream['id']}:{entry['skill']}:{key}:{rel_path}")

        self.assertEqual(missing, [])

    def test_every_packaged_skill_has_provenance_entry(self) -> None:
        data = self.load_provenance()
        local_skills = {
            path.name
            for path in (self.repo_root() / "plugins" / "unica" / "skills").iterdir()
            if path.is_dir()
        }
        indexed_skills = {
            entry["skill"]
            for upstream in data["upstreams"]
            for entry in upstream["entries"]
        }

        self.assertEqual(sorted(local_skills - indexed_skills), [])
        self.assertEqual(sorted(indexed_skills - local_skills), [])

    def test_upstream_review_records_real_drift_without_file_hashes(self) -> None:
        review = self.load_upstream_review()
        payload = json.dumps(review, ensure_ascii=False)
        upstreams = {item["id"]: item for item in review["upstreams"]}

        self.assertNotIn("sha256", payload)
        self.assertNotIn("Digest", payload)
        self.assertEqual(review["lastRefreshedAt"], "2026-06-22")
        self.assertEqual(
            upstreams["cc-1c-skills"]["targetCommit"],
            "3d36c2026916d2ae8915f0aca0836d55e1ccaabe",
        )
        self.assertEqual(upstreams["cc-1c-skills"]["commitsSinceBaseline"], 564)
        self.assertEqual(upstreams["cc-1c-skills"]["changedWatchedPathCount"], 62)
        self.assertEqual(len(upstreams["cc-1c-skills"]["affectedEntries"]), 31)
        self.assertNotIn("web-test", upstreams["cc-1c-skills"]["affectedEntries"])
        for skill in (
            "form-add",
            "form-compile",
            "form-edit",
            "skd-compile",
            "skd-edit",
            "meta-compile",
        ):
            self.assertIn(skill, upstreams["cc-1c-skills"]["affectedEntries"])
        for skill in (
            "cf-info",
            "form-info",
            "meta-info",
            "mxl-info",
            "role-info",
            "skd-info",
            "subsystem-info",
            "skd-validate",
            "form-patterns",
            "form-remove",
            "form-validate",
        ):
            self.assertNotIn(skill, upstreams["cc-1c-skills"]["affectedEntries"])
        self.assertIn("web-test", upstreams["cc-1c-skills"]["reviewedEntries"])
        for skill in (
            "skd-compile",
            "skd-edit",
            "skd-info",
            "skd-validate",
            "form-add",
            "form-compile",
            "form-edit",
            "form-info",
            "form-patterns",
            "form-remove",
            "form-validate",
        ):
            self.assertIn(skill, upstreams["cc-1c-skills"]["reviewedEntries"])
        web_test_decision = next(
            item
            for item in upstreams["cc-1c-skills"]["entryDecisions"]
            if item["skill"] == "web-test"
        )
        self.assertEqual(web_test_decision["decision"], "ported")
        for skill in (
            "skd-compile",
            "skd-edit",
            "skd-info",
            "skd-validate",
            "form-add",
            "form-compile",
            "form-edit",
            "form-info",
            "form-patterns",
            "form-remove",
            "form-validate",
            "meta-compile",
            "meta-info",
            "cf-info",
            "mxl-info",
            "role-info",
            "subsystem-info",
        ):
            decision = next(
                item
                for item in upstreams["cc-1c-skills"]["entryDecisions"]
                if item["skill"] == skill
            )
            self.assertEqual(decision["decision"], "ported")
            if skill == "meta-compile":
                self.assertEqual(
                    decision["baselineCommit"],
                    "ae8241237753850307d94b10df93e5293e29dc74",
                )
                self.assertIn("Fresh support", decision["evidence"])
            if skill in (
                "cf-info",
                "form-info",
                "meta-info",
                "mxl-info",
                "role-info",
                "skd-info",
                "subsystem-info",
            ):
                self.assertEqual(
                    decision["baselineCommit"],
                    "3d36c2026916d2ae8915f0aca0836d55e1ccaabe",
                )
                self.assertIn("support-state", decision["evidence"])
        self.assertEqual(upstreams["ai-rules-1c"]["commitsSinceBaseline"], 23)
        self.assertIn("code-search", upstreams["ai-rules-1c"]["affectedEntries"])
        self.assertEqual(upstreams["v8-runner-rust"]["commitsSinceBaseline"], 0)
        self.assertEqual(upstreams["v8-runner-rust"]["reviewedCommits"], 3)
        self.assertEqual(upstreams["v8-runner-rust"]["reviewStatus"], "applied")
        self.assertEqual(upstreams["v8-runner-rust"]["affectedEntries"], [])
        self.assertIn("v8-runner", upstreams["v8-runner-rust"]["reviewedEntries"])

    def test_product_update_backlog_tracks_all_planned_product_batches(self) -> None:
        backlog = self.load_product_backlog()
        products = {item["id"]: item for item in backlog["products"]}

        self.assertEqual(backlog["generatedAt"], "2026-06-21")
        self.assertEqual(products["bsl-analyzer"]["locked"], "v0.2.37")
        self.assertEqual(products["bsl-analyzer"]["latest"], "v0.2.43")
        self.assertEqual(products["bsl-analyzer"]["status"], "needs-review")
        self.assertEqual(products["rlm-tools-bsl"]["locked"], "v1.24.0")
        self.assertEqual(products["rlm-tools-bsl"]["latest"], "v1.24.0")
        self.assertEqual(products["rlm-tools-bsl"]["status"], "applied")
        self.assertEqual(products["rlm-bsl-index"]["locked"], "v1.24.0")
        self.assertEqual(products["rlm-bsl-index"]["latest"], "v1.24.0")
        self.assertEqual(products["rlm-bsl-index"]["status"], "applied")
        self.assertEqual(products["v8-runner"]["locked"], "v0.5.1")
        self.assertEqual(products["v8-runner"]["latest"], "v0.5.1")
        self.assertEqual(products["v8-runner"]["status"], "applied")
        self.assertEqual(products["playwright"]["latest"], "1.61.0")
        self.assertEqual(products["lxml"]["latest"], "6.1.1")
        self.assertEqual(products["rust-compatible-lock-updates"]["updateCount"], 2)
        self.assertEqual(products["rust-compatible-lock-updates"]["status"], "needs-review")
        self.assertTrue(products["bsl-analyzer"]["contractGate"])
        self.assertTrue(products["rlm-bsl-index"]["contractGate"])

    def test_current_cc_1c_source_comments_are_covered(self) -> None:
        data = self.load_provenance()
        cc_entries = next(item for item in data["upstreams"] if item["id"] == "cc-1c-skills")["entries"]
        covered_paths = {
            path
            for entry in cc_entries
            for path in [*entry.get("localPaths", []), *entry.get("contractPaths", [])]
        }
        source_comment_paths = []
        roots = [
            self.repo_root() / "tests" / "fixtures" / "unica_mcp_script_parity" / "reference_skills",
            self.repo_root() / "plugins" / "unica" / "skills" / "help-add" / "scripts",
            self.repo_root() / "plugins" / "unica" / "skills" / "web-test" / "scripts",
        ]
        for root in roots:
            for path in root.rglob("*"):
                if path.is_file() and "https://github.com/Nikolay-Shirokov/cc-1c-skills" in path.read_text(
                    encoding="utf-8", errors="ignore"
                ):
                    source_comment_paths.append(path.relative_to(self.repo_root()).as_posix())

        self.assertGreater(len(source_comment_paths), 0)
        uncovered = [
            path
            for path in source_comment_paths
            if not any(path == covered or path.startswith(covered.rstrip("/") + "/") for covered in covered_paths)
        ]
        self.assertEqual(sorted(uncovered), [])

    def test_donor_urls_do_not_enter_prompt_visible_skills_or_references(self) -> None:
        forbidden = [
            "https://github.com/Nikolay-Shirokov/cc-1c-skills",
            "https://github.com/comol/ai_rules_1c",
            "https://github.com/alkoleft/v8-runner-rust",
        ]
        scanned_roots = [
            self.repo_root() / "plugins" / "unica" / "skills",
            self.repo_root() / "plugins" / "unica" / "references",
        ]
        violations = []
        for root in scanned_roots:
            for path in root.rglob("*"):
                if not path.is_file():
                    continue
                text = path.read_text(encoding="utf-8", errors="ignore")
                for token in forbidden:
                    if token in text:
                        violations.append(f"{path.relative_to(self.repo_root())}: {token}")

        self.assertEqual(violations, [])

    def test_check_command_reports_runtime_tool_contract_drift(self) -> None:
        module = load_upstream_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            remote = root / "remote"
            clone = root / "clone"
            module.run_git(["init", "--bare", str(remote)], cwd=root)
            module.run_git(["clone", str(remote), str(clone)], cwd=root)
            module.run_git(["config", "user.email", "test@example.invalid"], cwd=clone)
            module.run_git(["config", "user.name", "Test User"], cwd=clone)
            (clone / "README.md").write_text("baseline\n", encoding="utf-8")
            module.run_git(["add", "README.md"], cwd=clone)
            module.run_git(["commit", "-m", "baseline"], cwd=clone)
            module.run_git(["tag", "-a", "v0.1.0", "-m", "v0.1.0"], cwd=clone)
            (clone / "README.md").write_text("baseline\nnew contract flag\n", encoding="utf-8")
            module.run_git(["commit", "-am", "contract change"], cwd=clone)
            module.run_git(["tag", "-a", "v0.2.0", "-m", "v0.2.0"], cwd=clone)
            module.run_git(["push", "--tags", "origin", "HEAD"], cwd=clone)

            index_path = root / "skill-upstreams.json"
            index_path.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "upstreams": [
                            {
                                "id": "runner",
                                "repository": str(remote),
                                "trackingRef": "v0.2.0",
                                "role": "runtime-tool-contract",
                                "toolLockRef": "v8-runner",
                                "entries": [
                                    {
                                        "skill": "v8-runner",
                                        "localPaths": [],
                                        "upstreamPaths": ["README.md"],
                                        "contractPaths": [],
                                        "status": "adapted",
                                        "notes": "test fixture",
                                    }
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            lock_file = root / "tools.lock.json"
            lock_file.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "tools": [
                            {
                                "name": "v8-runner",
                                "repository": str(remote),
                                "sourceTag": "v0.1.0",
                                "sourceCommit": module.git_output(["rev-parse", "v0.1.0^{}"], cwd=clone),
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            locked_baseline = module.git_output(["rev-parse", "v0.1.0^{}"], cwd=clone)

            report = module.check_upstreams(root, index_path, root / "cache", lock_file=lock_file)

        self.assertEqual(report.errors, [])
        self.assertEqual(report.upstreams[0]["id"], "runner")
        self.assertEqual(report.upstreams[0]["baselineSource"], "toolLockRef:v8-runner")
        self.assertTrue(report.upstreams[0]["contractDrift"])
        self.assertIn("README.md", report.upstreams[0]["changedPaths"])
        self.assertEqual(
            report.upstreams[0]["entries"],
            [
                {
                    "skill": "v8-runner",
                    "status": "adapted",
                    "baseline": locked_baseline,
                    "baselineSource": "toolLockRef:v8-runner",
                    "decision": "needs-review",
                    "upstreamDrift": True,
                    "changedPaths": ["README.md"],
                }
            ],
        )

    def test_entry_baseline_override_closes_drift_for_one_skill_without_closing_whole_upstream(self) -> None:
        module = load_upstream_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            remote = root / "remote"
            clone = root / "clone"
            module.run_git(["init", "--bare", str(remote)], cwd=root)
            module.run_git(["clone", str(remote), str(clone)], cwd=root)
            module.run_git(["config", "user.email", "test@example.invalid"], cwd=clone)
            module.run_git(["config", "user.name", "Test User"], cwd=clone)
            (clone / "a.md").write_text("a baseline\n", encoding="utf-8")
            (clone / "b.md").write_text("b baseline\n", encoding="utf-8")
            module.run_git(["add", "a.md", "b.md"], cwd=clone)
            module.run_git(["commit", "-m", "baseline"], cwd=clone)
            baseline = module.git_output(["rev-parse", "HEAD"], cwd=clone)
            (clone / "a.md").write_text("a updated\n", encoding="utf-8")
            (clone / "b.md").write_text("b updated\n", encoding="utf-8")
            module.run_git(["commit", "-am", "upstream changes"], cwd=clone)
            target = module.git_output(["rev-parse", "HEAD"], cwd=clone)
            branch = module.git_output(["branch", "--show-current"], cwd=clone)
            module.run_git(["push", "origin", "HEAD"], cwd=clone)

            index_path = root / "skill-upstreams.json"
            index_path.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "upstreams": [
                            {
                                "id": "donor",
                                "repository": str(remote),
                                "trackingRef": branch,
                                "role": "guidance",
                                "baselineCommit": baseline,
                                "entries": [
                                    {
                                        "skill": "closed-skill",
                                        "baselineCommit": target,
                                        "localPaths": [],
                                        "upstreamPaths": ["a.md"],
                                        "contractPaths": [],
                                        "status": "adapted",
                                        "decision": "ported",
                                        "notes": "test fixture",
                                    },
                                    {
                                        "skill": "open-skill",
                                        "localPaths": [],
                                        "upstreamPaths": ["b.md"],
                                        "contractPaths": [],
                                        "status": "adapted",
                                        "notes": "test fixture",
                                    },
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            report = module.check_upstreams(root, index_path, root / "cache")

        self.assertEqual(report.errors, [])
        entries = {entry["skill"]: entry for entry in report.upstreams[0]["entries"]}
        self.assertFalse(entries["closed-skill"]["upstreamDrift"])
        self.assertEqual(entries["closed-skill"]["baseline"], target)
        self.assertEqual(entries["closed-skill"]["decision"], "ported")
        self.assertTrue(entries["open-skill"]["upstreamDrift"])
        self.assertEqual(entries["open-skill"]["baseline"], baseline)
        self.assertEqual(entries["open-skill"]["decision"], "needs-review")
        self.assertEqual(report.upstreams[0]["affectedEntries"], ["open-skill"])

    def test_prepare_upstream_review_has_no_checksums(self) -> None:
        module = load_upstream_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            remote = root / "remote"
            clone = root / "clone"
            module.run_git(["init", "--bare", str(remote)], cwd=root)
            module.run_git(["clone", str(remote), str(clone)], cwd=root)
            module.run_git(["config", "user.email", "test@example.invalid"], cwd=clone)
            module.run_git(["config", "user.name", "Test User"], cwd=clone)
            (clone / "README.md").write_text("baseline\n", encoding="utf-8")
            module.run_git(["add", "README.md"], cwd=clone)
            module.run_git(["commit", "-m", "baseline"], cwd=clone)
            baseline = module.git_output(["rev-parse", "HEAD"], cwd=clone)
            branch = module.git_output(["branch", "--show-current"], cwd=clone)
            (clone / "README.md").write_text("baseline\nnew guidance\n", encoding="utf-8")
            module.run_git(["commit", "-am", "guidance change"], cwd=clone)
            module.run_git(["push", "origin", "HEAD"], cwd=clone)

            index_path = root / "skill-upstreams.json"
            index_path.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "upstreams": [
                            {
                                "id": "guidance",
                                "repository": str(remote),
                                "trackingRef": branch,
                                "role": "guidance",
                                "baselineCommit": baseline,
                                "entries": [
                                    {
                                        "skill": "code-search",
                                        "localPaths": [],
                                        "upstreamPaths": ["README.md"],
                                        "contractPaths": [],
                                        "status": "adapted",
                                        "notes": "test fixture",
                                    }
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            review = module.prepare_upstream_review(root, index_path, root / "cache")

        payload = json.dumps(review, ensure_ascii=False)
        self.assertNotIn("sha256", payload)
        self.assertNotIn("Digest", payload)
        self.assertEqual(review["upstreams"][0]["reviewStatus"], "needs-review")
        self.assertEqual(review["upstreams"][0]["affectedEntries"], ["code-search"])
        self.assertEqual(review["upstreams"][0]["entries"][0]["decision"], "needs-review")


if __name__ == "__main__":
    unittest.main()
