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
        return self.repo_root() / "spec" / "provenance" / "skill-upstreams.json"

    def reviews_dir(self) -> Path:
        return self.repo_root() / "docs" / "provenance" / "reviews"

    def upstream_review_path(self) -> Path:
        return self.reviews_dir() / "2026-06-15-upstream-review.json"

    def product_backlog_path(self) -> Path:
        return self.reviews_dir() / "2026-07-29-product-update-backlog.json"

    def load_provenance(self) -> dict:
        return json.loads(self.provenance_path().read_text(encoding="utf-8"))

    def load_upstream_review(self) -> dict:
        return json.loads(self.upstream_review_path().read_text(encoding="utf-8"))

    def load_product_backlog(self) -> dict:
        return json.loads(self.product_backlog_path().read_text(encoding="utf-8"))

    def test_adapted_python_models_are_named_as_unica_owned_test_models(self) -> None:
        root = (
            self.repo_root()
            / "tests"
            / "fixtures"
            / "unica_mcp_script_parity"
            / "unica_reference_models"
        )
        self.assertTrue(root.is_dir())
        self.assertFalse(
            (
                self.repo_root()
                / "tests"
                / "fixtures"
                / "unica_mcp_script_parity"
                / "reference_skills"
            ).exists()
        )
        python_models = sorted(root.glob("*/scripts/*.py"))
        self.assertGreater(len(python_models), 0)
        for path in python_models:
            text = path.read_text(encoding="utf-8", errors="ignore")
            self.assertIn(
                "Adapted from https://github.com/Nikolay-Shirokov/cc-1c-skills",
                text,
                path,
            )

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
            subprocess.run(
                ["git", "-c", "commit.gpgsign=false", "commit", "-m", "stale"],
                cwd=repo,
                check=True,
                stdout=subprocess.PIPE,
            )
            stale_commit = module.git_output(["rev-parse", "HEAD"], cwd=repo)

            marker.write_text("fresh\n", encoding="utf-8")
            subprocess.run(
                ["git", "-c", "commit.gpgsign=false", "commit", "-am", "fresh"],
                cwd=repo,
                check=True,
                stdout=subprocess.PIPE,
            )
            fresh_commit = module.git_output(["rev-parse", "HEAD"], cwd=repo)
            subprocess.run(["git", "update-ref", "refs/remotes/origin/main", fresh_commit], cwd=repo, check=True)
            subprocess.run(["git", "reset", "--hard", stale_commit], cwd=repo, check=True, stdout=subprocess.PIPE)

            self.assertEqual(module.git_output(["rev-parse", "main"], cwd=repo), stale_commit)
            self.assertEqual(module.git_output(["rev-parse", "origin/main"], cwd=repo), fresh_commit)
            self.assertEqual(module.resolve_ref(repo, "main"), fresh_commit)

    def test_provenance_index_lives_outside_the_package(self) -> None:
        """Maintainer metadata is a source-tree artifact, not a shipped file.

        The index exists to check `ATTRIBUTIONS.md` for completeness against
        the donor inventory; that check runs over the source tree and never in
        a consumer's install. The licence obligation is discharged by the
        notice itself, which names every upstream.
        """
        path = self.provenance_path()

        self.assertTrue(path.is_file())
        self.assertIn("spec/provenance", path.as_posix())
        self.assertNotIn("plugins/unica", path.as_posix())
        self.assertFalse((self.repo_root() / "plugins" / "unica" / "provenance").exists())

    def test_review_records_live_in_the_archive_tree(self) -> None:
        reviews = self.reviews_dir()

        self.assertTrue(reviews.is_dir())
        self.assertIn("docs/provenance/reviews", reviews.as_posix())
        self.assertNotIn("plugins/unica", reviews.as_posix())

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

    def test_templates_new_object_scope_names_every_adopted_convention(self) -> None:
        data = self.load_provenance()
        upstream = next(
            item for item in data["upstreams"] if item["id"] == "templates-new-object-1c"
        )
        entry = next(item for item in upstream["entries"] if item["skill"] == "meta-add")
        notes = entry["notes"].lower()

        for phrase in (
            "naming",
            "synonym",
            "representation",
            "fill-check",
            "catalog code",
            "information-register command-interface",
        ):
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, notes)

    def test_templates_new_object_records_source_scope_and_unica_adoption(self) -> None:
        data = self.load_provenance()
        upstream = next(
            item for item in data["upstreams"] if item["id"] == "templates-new-object-1c"
        )
        entry = next(item for item in upstream["entries"] if item["skill"] == "meta-add")
        notes = entry["notes"].lower()

        self.assertIn("1c:accounting", notes)
        self.assertIn("other configurations may differ", notes)
        self.assertIn("general unica project conventions", notes)
        self.assertIn("not platform requirements", notes)

        attribution = (
            self.repo_root() / "plugins" / "unica" / "ATTRIBUTIONS.md"
        ).read_text(encoding="utf-8")
        reference = (
            self.repo_root()
            / "plugins"
            / "unica"
            / "references"
            / "platform"
            / "metadata-conventions.md"
        ).read_text(encoding="utf-8")

        for raw_text in (attribution, reference):
            text = " ".join(raw_text.split())
            self.assertIn("«1С:Бухгалтерии предприятия»", text)
            self.assertIn("общие проектные соглашения Unica", text)
            self.assertIn("не требования платформы", text)

    def test_retired_meta_donors_have_typed_or_internal_local_owners(self) -> None:
        data = self.load_provenance()
        cc = next(item for item in data["upstreams"] if item["id"] == "cc-1c-skills")
        creation = next(item for item in cc["entries"] if item["skill"] == "meta-add")
        info_entries = [item for item in cc["entries"] if item["skill"] == "meta-info"]

        self.assertIn(".claude/skills/meta-compile/**", creation["upstreamPaths"])
        self.assertIn("tests/skills/cases/meta-compile/**", creation["upstreamPaths"])
        self.assertNotIn("donorScope", creation)
        self.assertEqual(len(info_entries), 1)
        info = info_entries[0]
        self.assertNotIn("componentOwner", info)
        self.assertNotIn("donorScope", info)
        self.assertIn(".claude/skills/meta-info/**", info["upstreamPaths"])
        self.assertIn(".claude/skills/meta-validate/**", info["upstreamPaths"])
        self.assertIn(
            "crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs",
            info["localPaths"],
        )

        baseline = json.loads(
            (
                self.repo_root()
                / "tests"
                / "fixtures"
                / "unica_mcp_script_parity"
                / "donor-baseline.json"
            ).read_text(encoding="utf-8")
        )
        validation_scope = baseline["scopes"]["meta-validate"]
        self.assertEqual(validation_scope["ownerSkill"], "meta-info")
        self.assertEqual(
            info["parityBaselineCommit"],
            validation_scope["acceptedCommit"],
        )

        archived = self.repo_root() / "tests" / "fixtures" / "provenance" / "retired_meta_dsl"
        self.assertTrue((archived / "meta-compile").is_dir())
        self.assertTrue((archived / "meta-edit").is_dir())
        self.assertTrue((archived / "meta-validate").is_dir())
        active_models = (
            self.repo_root()
            / "tests"
            / "fixtures"
            / "unica_mcp_script_parity"
            / "unica_reference_models"
        )
        for retired in ("meta-compile", "meta-edit", "meta-validate"):
            self.assertFalse((active_models / retired).exists())

    def test_checker_rejects_duplicate_skill_entries_within_one_upstream(self) -> None:
        module = load_upstream_module()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            index_path = root / "skill-upstreams.json"
            index_path.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "upstreams": [
                            {
                                "id": "donor",
                                "repository": "https://example.invalid/donor.git",
                                "trackingRef": "main",
                                "role": "guidance",
                                "baselineCommit": "1" * 40,
                                "entries": [
                                    {
                                        "skill": "meta-info",
                                        "status": "adapted",
                                        "notes": "typed reader",
                                        "localPaths": [],
                                    },
                                    {
                                        "skill": "meta-info",
                                        "status": "adapted",
                                        "notes": "internal validator",
                                        "localPaths": [],
                                    },
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            report = module.validate_index(root, index_path)

        self.assertTrue(
            any("duplicate skill entry" in error and "meta-info" in error for error in report.errors),
            report.errors,
        )

    def test_general_and_parity_baselines_are_independent_concrete_commits(self) -> None:
        data = self.load_provenance()
        upstreams = {item["id"]: item for item in data["upstreams"]}
        cc = upstreams["cc-1c-skills"]
        self.assertRegex(cc["baselineCommit"], r"^[0-9a-f]{40}$")
        self.assertRegex(
            cc["lastAdaptedLocalCommit"], r"^[0-9a-f]{40}$"
        )
        baseline = json.loads(
            (
                self.repo_root()
                / "tests"
                / "fixtures"
                / "unica_mcp_script_parity"
                / "donor-baseline.json"
            ).read_text(encoding="utf-8")
        )
        entries = {entry["skill"]: entry for entry in cc["entries"]}
        for scope, scope_data in baseline["scopes"].items():
            with self.subTest(scope=scope):
                commit = scope_data["acceptedCommit"]
                self.assertRegex(commit, r"^[0-9a-f]{40}$")
                owner = scope_data["ownerSkill"]
                self.assertEqual(entries[owner]["parityBaselineCommit"], commit)
                self.assertNotEqual(entries[owner]["baselineCommit"], commit)
                review = json.loads(
                    (
                        self.reviews_dir()
                        / f"{scope_data['reviewId']}.json"
                    ).read_text(encoding="utf-8")
                )
                self.assertEqual(review["reviewStatus"], "reviewed")
                self.assertTrue(review["applied"])
                self.assertEqual(review["targetCommit"], commit)

        self.assertEqual(
            upstreams["ai-rules-1c"]["baselineCommit"],
            "484e550043a4cb749d59d0671329f3112e3ae668",
        )
        self.assertEqual(
            upstreams["ai-rules-1c"]["lastReviewedLocalCommit"],
            "e5b4eeab4dac92e0c9f60d3f886aa2bb7ef79f80",
        )

    def test_api_design_is_unica_owned_not_ai_rules_primary_source(self) -> None:
        data = self.load_provenance()
        ai_rules = next(item for item in data["upstreams"] if item["id"] == "ai-rules-1c")
        api_design = next(entry for entry in ai_rules["entries"] if entry["skill"] == "api-design")

        self.assertEqual(api_design["primarySource"], "unica")
        self.assertEqual(api_design["decision"], "ignored-with-reason")
        self.assertIn("Unica-owned", api_design["decisionReason"])
        self.assertIn("general ideas", api_design["notes"])
        self.assertIn("no donor expression", api_design["notes"])

    def test_v8_runner_license_matches_pinned_source_and_is_packaged(self) -> None:
        tool_lock = json.loads(
            (self.repo_root() / "plugins/unica/third-party/tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        runner = next(tool for tool in tool_lock["tools"] if tool["name"] == "v8-runner")

        self.assertEqual(runner["license"], "AGPL-3.0-only")
        license_path = self.repo_root() / "plugins/unica/third-party/licenses/v8-runner/LICENSE"
        self.assertTrue(license_path.is_file())
        self.assertIn("GNU AFFERO GENERAL PUBLIC LICENSE", license_path.read_text(encoding="utf-8"))

    def test_ai_rules_is_recorded_as_inspiration_not_adaptation(self) -> None:
        ai_rules = next(
            item for item in self.load_provenance()["upstreams"] if item["id"] == "ai-rules-1c"
        )

        self.assertEqual(ai_rules.get("usage"), "inspiration-only")
        self.assertNotIn("lastAdaptedLocalCommit", ai_rules)
        self.assertNotIn("lastAdaptedAt", ai_rules)
        self.assertIn("lastReviewedLocalCommit", ai_rules)
        for entry in ai_rules["entries"]:
            self.assertEqual(entry["status"], "inspiration-only")
            self.assertEqual(entry["primarySource"], "unica")
            self.assertEqual(entry["decision"], "ignored-with-reason")
            self.assertIn("ideas", entry["decisionReason"])

    def test_code_patch_is_recorded_as_exclusively_unica_owned(self) -> None:
        data = self.load_provenance()
        owned = {entry["skill"]: entry for entry in data["unicaOwnedSkills"]}
        donor_skills = {
            entry["skill"]
            for upstream in data["upstreams"]
            for entry in upstream["entries"]
        }

        self.assertIn("code-patch", owned)
        self.assertNotIn("code-patch", donor_skills)
        self.assertEqual(
            owned["code-patch"]["localPaths"],
            ["plugins/unica/skills/code-patch"],
        )
        self.assertNotIn("repository", owned["code-patch"])
        self.assertNotIn("upstreamPaths", owned["code-patch"])
        self.assertNotIn("baselineCommit", owned["code-patch"])

    def test_source_access_is_recorded_as_exclusively_unica_owned(self) -> None:
        data = self.load_provenance()
        owned = {entry["skill"]: entry for entry in data["unicaOwnedSkills"]}
        donor_skills = {
            entry["skill"]
            for upstream in data["upstreams"]
            for entry in upstream["entries"]
        }

        self.assertIn("source-access", owned)
        self.assertNotIn("source-access", donor_skills)
        self.assertEqual(
            owned["source-access"]["localPaths"],
            ["plugins/unica/skills/source-access"],
        )
        self.assertNotIn("repository", owned["source-access"])
        self.assertNotIn("upstreamPaths", owned["source-access"])
        self.assertNotIn("baselineCommit", owned["source-access"])

    def test_xdto_records_adapted_donor_material_and_mcp_first_divergence(self) -> None:
        data = self.load_provenance()
        owned = {entry["skill"] for entry in data["unicaOwnedSkills"]}
        cc = next(item for item in data["upstreams"] if item["id"] == "cc-1c-skills")
        entries = {item["skill"]: item for item in cc["entries"]}
        self.assertIn("xdto", entries)
        entry = entries["xdto"]

        self.assertNotIn("xdto", owned)
        self.assertEqual(entry["status"], "adapted")
        self.assertEqual(entry["decision"], "ported")
        self.assertEqual(
            entry["baselineCommit"],
            "2067778ba3bad527bd1e5850304d1c82acb81fc8",
        )
        self.assertEqual(
            set(entry["upstreamPaths"]),
            {
                "docs/xdto-guide.md",
                "docs/xdto-dsl-spec.md",
                ".claude/skills/xdto-compile/**",
                ".claude/skills/xdto-decompile/**",
                ".claude/skills/xdto-edit/**",
                ".claude/skills/xdto-info/**",
                ".claude/skills/xdto-validate/**",
                "tests/skills/cases/xdto-compile/**",
                "tests/skills/cases/xdto-decompile/**",
                "tests/skills/cases/xdto-edit/**",
                "tests/skills/cases/xdto-info/**",
                "tests/skills/cases/xdto-validate/**",
            },
        )
        self.assertEqual(
            set(entry["localPaths"]),
            {
                "plugins/unica/skills/xdto",
                "plugins/unica/references/specs/1c-xdto-spec.md",
                "tests/fixtures/xdto/enterprise-data-minimal",
            },
        )
        notes = entry["notes"]
        self.assertIn("adapted", notes)
        self.assertIn("MCP-first", notes)
        self.assertIn("unica.xdto.info", notes)
        self.assertIn("unica.xdto.edit", notes)
        for excluded_route in ("compile", "decompile", "validate", "script wrappers"):
            with self.subTest(excluded_route=excluded_route):
                self.assertIn(excluded_route, notes)

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
        self.assertEqual(locked_tools["v8-runner"]["sourceTag"], "master")
        self.assertEqual(
            locked_tools["v8-runner"]["sourceCommit"],
            "7ce1b062843d86644fe55741dbe0ee79f7ca767d",
        )

    def test_rlm_tools_are_locked_to_reviewed_1_29_1_pair(self) -> None:
        tool_lock = json.loads(
            (self.repo_root() / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        locked_tools = {tool["name"]: tool for tool in tool_lock["tools"]}

        for name in ("rlm-tools-bsl", "rlm-bsl-index"):
            self.assertEqual(locked_tools[name]["version"], "1.29.1")
            self.assertEqual(locked_tools[name]["sourceTag"], "v1.29.1")
            self.assertEqual(
                locked_tools[name]["sourceCommit"],
                "8bc6e9fc83b522f9a79eab3193eb13fc2cecb8ed",
            )
            self.assertEqual(
                locked_tools[name]["assetTag"],
                "rlm-tools-bsl-v1.29.1-build.2",
            )

        self.assertEqual(
            locked_tools["rlm-tools-bsl"]["assets"],
            {
                "darwin-arm64": {
                    "assetName": "rlm-tools-bsl-darwin-arm64",
                    "sha256": "4a1cd5c2fc0c6c27f049241a4008dbe382a7d23ab01b5e9cfdc91a75d9eaba65",
                },
                "linux-x64": {
                    "assetName": "rlm-tools-bsl-linux-x64",
                    "sha256": "dec0334cb640ee94d97b80ff3d0c8e4c39e4426eceffbfe932378526876c4417",
                },
                "win-x64": {
                    "assetName": "rlm-tools-bsl-win-x64.exe",
                    "sha256": "349d6002ecf551f1ab99e24aa097aeb207087acf9de8ab4adef42c2b7eaf6539",
                },
            },
        )
        self.assertEqual(
            locked_tools["rlm-bsl-index"]["assets"],
            {
                "darwin-arm64": {
                    "assetName": "rlm-bsl-index-darwin-arm64",
                    "sha256": "b20725360b889944547cb2b1823df7ce8bc4b6b39c103debb602d572648d42ad",
                },
                "linux-x64": {
                    "assetName": "rlm-bsl-index-linux-x64",
                    "sha256": "5e68d6048ad384df36a54a7edf0f4fd0c89cd583cf978cc91d7144cd5f788a5d",
                },
                "win-x64": {
                    "assetName": "rlm-bsl-index-win-x64.exe",
                    "sha256": "e72ddea7ecc841800a3dde479ac4ef1680f7ea0ca60c9cd75004e727fa939cef",
                },
            },
        )

    def test_bsl_analyzer_contract_is_v0_2_62(self) -> None:
        tool_lock = json.loads(
            (self.repo_root() / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        locked_tools = {tool["name"]: tool for tool in tool_lock["tools"]}

        analyzer = locked_tools["bsl-analyzer"]
        self.assertEqual(analyzer["version"], "0.2.62")
        self.assertEqual(analyzer["sourceTag"], "v0.2.62")
        self.assertEqual(
            analyzer["sourceCommit"],
            "9a6cb15d60c0381dce6a3b5e536434adb12da89b",
        )
        self.assertEqual(analyzer["assetTag"], "bsl-analyzer-v0.2.62-build.1")
        self.assertEqual(
            {target: asset["sha256"] for target, asset in analyzer["assets"].items()},
            {
                "darwin-arm64": "97c599b2be9e8c4e267d7a8567b21d01d5d6939060d28084ae1f598d15c084a4",
                "linux-x64": "070374453c933025c0750d59a658dfe9edd6415f7b5aa80d122268acc08ae8b9",
                "win-x64": "9c42ef7d6b379b3f80afb525f9cbec757abd2ba1877bbdcfb5db49df9972fd22",
            },
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
        indexed_skills.update(entry["skill"] for entry in data.get("unicaOwnedSkills", []))

        self.assertEqual(sorted(local_skills - indexed_skills), [])
        self.assertEqual(sorted(indexed_skills - local_skills), [])

    def test_upstream_review_records_real_drift_without_file_hashes(self) -> None:
        review = self.load_upstream_review()
        payload = json.dumps(review, ensure_ascii=False)
        upstreams = {item["id"]: item for item in review["upstreams"]}

        self.assertNotIn("sha256", payload)
        self.assertNotIn("Digest", payload)
        self.assertEqual(review["lastRefreshedAt"], "2026-07-04")
        self.assertEqual(
            upstreams["cc-1c-skills"]["targetCommit"],
            "78b5b73fa7f835462dc4073ae7a9fc841e7c62fb",
        )
        self.assertEqual(upstreams["cc-1c-skills"]["commitsSinceBaseline"], 607)
        self.assertEqual(upstreams["cc-1c-skills"]["changedWatchedPathCount"], 0)
        self.assertEqual(upstreams["cc-1c-skills"]["affectedEntries"], [])
        previous_target = "cbde49efdaeec190432fdf4a53201a87e83c69de"
        target = "78b5b73fa7f835462dc4073ae7a9fc841e7c62fb"
        historical_dcs_edit = "s" + "kd-edit"
        functional_skills = {
            "form-remove",
            historical_dcs_edit,
            "subsystem-compile",
        }
        historical_script_backed_skills = {"img-grid", "web-test"}
        previous_functional_skills = {"cfe-borrow", "cfe-init", "form-validate"}
        decisions = {
            item["skill"]: item
            for item in upstreams["cc-1c-skills"]["entryDecisions"]
        }

        self.assertGreaterEqual(len(decisions), 40)
        for skill in functional_skills:
            self.assertIn(skill, upstreams["cc-1c-skills"]["reviewedEntries"])
            self.assertEqual(decisions[skill]["decision"], "ported")
            self.assertEqual(decisions[skill]["baselineCommit"], target)

        for skill in previous_functional_skills:
            self.assertIn(skill, upstreams["cc-1c-skills"]["reviewedEntries"])
            self.assertEqual(decisions[skill]["decision"], "ported")
            self.assertEqual(decisions[skill]["baselineCommit"], previous_target)

        self.assertIn("Default*Form", decisions["form-remove"]["evidence"])
        self.assertIn("expr_start", decisions[historical_dcs_edit]["evidence"])
        self.assertIn("subprocess.run", decisions["subsystem-compile"]["evidence"])
        self.assertIn("BorrowMainAttribute", decisions["cfe-borrow"]["evidence"])
        self.assertIn("MDClasses format version", decisions["cfe-init"]["evidence"])
        self.assertIn("type_error_count", decisions["form-validate"]["evidence"])

        for skill in historical_script_backed_skills:
            self.assertEqual(decisions[skill]["decision"], "script-backed-utility-exception")

        removal = json.loads(
            (
                self.reviews_dir() / "2026-07-20-script-backed-skill-removal.json"
            ).read_text(encoding="utf-8")
        )
        self.assertEqual(set(removal["removedSkills"]), historical_script_backed_skills)
        self.assertEqual(removal["decision"], "removed-from-product")

        ignored_skills = (
            set(decisions)
            - functional_skills
            - previous_functional_skills
            - historical_script_backed_skills
        )
        self.assertIn("cf-edit", ignored_skills)
        self.assertIn("epf-bsp-init", ignored_skills)
        self.assertIn("help-add", ignored_skills)
        for skill in ignored_skills:
            decision = decisions[skill]
            self.assertIn(skill, upstreams["cc-1c-skills"]["reviewedEntries"])
            self.assertEqual(decision["decision"], "ignored-with-reason")
            self.assertEqual(decision["baselineCommit"], previous_target)
            self.assertIn("EOL", decision["evidence"])
            self.assertIn("donor-only", decision["evidence"])
        self.assertEqual(upstreams["ai-rules-1c"]["commitsSinceBaseline"], 23)
        self.assertEqual(upstreams["ai-rules-1c"]["changedWatchedPathCount"], 0)
        self.assertEqual(upstreams["ai-rules-1c"]["affectedEntries"], [])
        self.assertEqual(upstreams["ai-rules-1c"]["reviewStatus"], "reviewed")
        self.assertIn("api-design", upstreams["ai-rules-1c"]["reviewedEntries"])
        self.assertNotIn("api-design", upstreams["ai-rules-1c"]["affectedEntries"])
        ai_rules_decisions = {
            item["skill"]: item
            for item in upstreams["ai-rules-1c"]["entryDecisions"]
        }
        self.assertEqual(ai_rules_decisions["api-design"]["decision"], "ignored-with-reason")
        self.assertEqual(ai_rules_decisions["api-design"]["primarySource"], "unica")
        self.assertIn("Unica-owned", ai_rules_decisions["api-design"]["evidence"])
        self.assertEqual(ai_rules_decisions["code-search"]["decision"], "ported")
        self.assertIn("MCP-first", ai_rules_decisions["code-search"]["evidence"])
        self.assertEqual(upstreams["v8-runner-rust"]["commitsSinceBaseline"], 0)
        self.assertEqual(upstreams["v8-runner-rust"]["reviewedCommits"], 3)
        self.assertEqual(upstreams["v8-runner-rust"]["reviewStatus"], "applied")
        self.assertEqual(upstreams["v8-runner-rust"]["affectedEntries"], [])
        self.assertIn("v8-runner", upstreams["v8-runner-rust"]["reviewedEntries"])

    def test_product_update_backlog_tracks_all_planned_product_batches(self) -> None:
        backlog = self.load_product_backlog()
        products = {item["id"]: item for item in backlog["products"]}

        self.assertEqual(backlog["generatedAt"], "2026-07-29")
        tool_lock = json.loads(
            (self.repo_root() / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        locked_tools = {tool["name"]: tool for tool in tool_lock["tools"]}
        analyzer_tag = locked_tools["bsl-analyzer"]["sourceTag"]

        self.assertEqual(products["bsl-analyzer"]["locked"], analyzer_tag)
        self.assertEqual(products["bsl-analyzer"]["latest"], analyzer_tag)
        self.assertEqual(products["bsl-analyzer"]["status"], "applied")
        self.assertEqual(products["rlm-tools-bsl"]["locked"], "v1.29.1")
        self.assertEqual(products["rlm-tools-bsl"]["latest"], "v1.29.1")
        self.assertEqual(products["rlm-tools-bsl"]["status"], "applied")
        self.assertEqual(products["rlm-bsl-index"]["locked"], "v1.29.1")
        self.assertEqual(products["rlm-bsl-index"]["latest"], "v1.29.1")
        self.assertEqual(products["rlm-bsl-index"]["status"], "applied")
        self.assertEqual(products["v8-runner"]["locked"], "v0.5.1")
        self.assertEqual(products["v8-runner"]["latest"], "v0.5.1")
        self.assertEqual(products["v8-runner"]["status"], "applied")
        self.assertEqual(products["lxml"]["latest"], "6.1.1")
        self.assertEqual(products["rust-compatible-lock-updates"]["updateCount"], 4)
        self.assertEqual(products["rust-compatible-lock-updates"]["status"], "applied")
        self.assertEqual(products["serde-yaml-replacement"]["status"], "deferred")
        self.assertEqual(products["ureq-3"]["status"], "deferred")
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
            self.repo_root() / "tests" / "fixtures" / "unica_mcp_script_parity" / "unica_reference_models",
            self.repo_root() / "tests" / "fixtures" / "provenance" / "retired_meta_dsl",
            self.repo_root() / "plugins" / "unica" / "skills" / "help-add" / "scripts",
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

    def test_donor_case_scopes_are_watched_by_provenance(self) -> None:
        data = self.load_provenance()
        cc = next(
            item for item in data["upstreams"] if item["id"] == "cc-1c-skills"
        )
        entries = {entry["skill"]: entry for entry in cc["entries"]}
        expected = {
            "cfe-borrow": ["tests/skills/cases/cfe-borrow/**"],
            "dcs-compile": ["tests/skills/cases/skd-compile/**"],
            "form-compile": [
                "tests/skills/cases/form-compile/**",
                "tests/skills/cases/form-compile-from-object/**",
            ],
            "meta-add": ["tests/skills/cases/meta-compile/**"],
        }
        for skill, paths in expected.items():
            with self.subTest(skill=skill):
                for path in paths:
                    self.assertIn(path, entries[skill]["upstreamPaths"])

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

    def test_unica_primary_source_entry_can_ignore_secondary_guidance_drift(self) -> None:
        module = load_upstream_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            remote = root / "remote"
            clone = root / "clone"
            module.run_git(["init", "--bare", str(remote)], cwd=root)
            module.run_git(["clone", str(remote), str(clone)], cwd=root)
            module.run_git(["config", "user.email", "test@example.invalid"], cwd=clone)
            module.run_git(["config", "user.name", "Test User"], cwd=clone)
            (clone / "api.md").write_text("baseline\n", encoding="utf-8")
            module.run_git(["add", "api.md"], cwd=clone)
            module.run_git(["commit", "-m", "baseline"], cwd=clone)
            baseline = module.git_output(["rev-parse", "HEAD"], cwd=clone)
            (clone / "api.md").write_text("donor update\n", encoding="utf-8")
            module.run_git(["commit", "-am", "secondary guidance"], cwd=clone)
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
                                        "skill": "api-design",
                                        "primarySource": "unica",
                                        "localPaths": [],
                                        "upstreamPaths": ["api.md"],
                                        "contractPaths": [],
                                        "status": "adapted",
                                        "decision": "ignored-with-reason",
                                        "decisionReason": "Unica-owned skill; donor update is secondary guidance only.",
                                        "notes": "Unica-owned skill; donor is secondary guidance only.",
                                    }
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            report = module.check_upstreams(root, index_path, root / "cache")

        self.assertEqual(report.errors, [])
        upstream = report.upstreams[0]
        entry = upstream["entries"][0]
        self.assertEqual(entry["primarySource"], "unica")
        self.assertEqual(entry["decision"], "ignored-with-reason")
        self.assertFalse(entry["upstreamDrift"])
        self.assertEqual(entry["changedPaths"], [])
        self.assertEqual(upstream["affectedEntries"], [])

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
