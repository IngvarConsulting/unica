from __future__ import annotations

import copy
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path


RLM_STANDALONE_TARGET_EVIDENCE = {
    "darwin-arm64": {
        "archive": {
            "assetName": "rlm-tools-bsl-darwin-arm64.tar.gz",
            "sha256": "55caf6a245b3bb47344e2191408841f45aefb614b23480d9941f2cb3e2d8af2c",
            "size": 72_708_783,
        },
        "checksumAsset": "checksums-rlm-tools-bsl-darwin-arm64.txt",
        "provenanceAsset": "provenance-rlm-tools-bsl-darwin-arm64.json",
        "runtimeManifestSha256": "d4b032d798857a5ab752953c562f13e974906438d169b95f6881a8caf0cb639f",
        "payload": {
            "fileCount": 79,
            "size": 239_958_372,
            "entrypointSha256": "bdf429e3a8dee1fb9b1f1af66adcc4280732cc4287c92c4fbe4effddc0f8492e",
        },
        "builder": {
            "kind": "python-nuitka-standalone",
            "python": "3.12.10",
            "uv": "0.11.29",
            "nuitka": "4.1.3",
            "compiler": {
                "cCompiler": "Clang",
                "ccName": "clang",
                "compiler": "clang",
            },
        },
        "entrypoints": {
            "rlm-bsl-index": "rlm-bsl-index",
            "rlm-bsl-mcp": "rlm-bsl-mcp",
        },
    },
    "linux-x64": {
        "archive": {
            "assetName": "rlm-tools-bsl-linux-x64.tar.gz",
            "sha256": "1a27e1305c159c01f4b928fa63358567236197844af4663188bd5b30aa780f40",
            "size": 106_083_876,
        },
        "checksumAsset": "checksums-rlm-tools-bsl-linux-x64.txt",
        "provenanceAsset": "provenance-rlm-tools-bsl-linux-x64.json",
        "runtimeManifestSha256": "51ee9f88787f0d51305ab9056ccf3447a14b47d0a23f189ff3a52d22172400cc",
        "payload": {
            "fileCount": 83,
            "size": 295_193_488,
            "entrypointSha256": "d63ea531fb67c30f921e59333ec009cd45df99f9d35d24bd2345f1a023d83027",
        },
        "builder": {
            "kind": "python-nuitka-standalone",
            "python": "3.12.10",
            "uv": "0.11.29",
            "nuitka": "4.1.3",
            "compiler": {"cCompiler": "gcc", "ccName": "gcc", "compiler": "gcc"},
        },
        "entrypoints": {
            "rlm-bsl-index": "rlm-bsl-index",
            "rlm-bsl-mcp": "rlm-bsl-mcp",
        },
    },
    "win-x64": {
        "archive": {
            "assetName": "rlm-tools-bsl-win-x64.tar.gz",
            "sha256": "9655a8d052ae3d033ea8761e7a503ffd1d9a7e4f303b17ed6c8bc9fd86e5abb2",
            "size": 75_235_914,
        },
        "checksumAsset": "checksums-rlm-tools-bsl-win-x64.txt",
        "provenanceAsset": "provenance-rlm-tools-bsl-win-x64.json",
        "runtimeManifestSha256": "fb2f6e6b2d64a9b55a355242d16e12933a4023e926ce300dea8e6851ac29da59",
        "payload": {
            "fileCount": 62,
            "size": 224_722_940,
            "entrypointSha256": "dd7f12096475eca857d6d144eaf9bd7af3446690f0cef4a0e2c6ac5ba4dee5b3",
        },
        "builder": {
            "kind": "python-nuitka-standalone",
            "python": "3.12.10",
            "uv": "0.11.29",
            "nuitka": "4.1.3",
            "compiler": {"cCompiler": "MSVC", "ccName": "cl", "compiler": "cl"},
        },
        "entrypoints": {
            "rlm-bsl-index": "rlm-bsl-index.exe",
            "rlm-bsl-mcp": "rlm-bsl-mcp.exe",
        },
    },
}


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
        return self.repo_root() / "docs" / "provenance" / "skill-upstreams.json"

    def reviews_dir(self) -> Path:
        return self.repo_root() / "docs" / "provenance" / "reviews"

    def upstream_review_path(self) -> Path:
        return self.reviews_dir() / "2026-06-15-upstream-review.json"

    def product_backlog_path(self) -> Path:
        return self.reviews_dir() / "2026-08-12-product-update-backlog.json"

    def rlm_standalone_review_path(self) -> Path:
        return (
            self.reviews_dir()
            / "2026-08-14-rlm-v1-33-nuitka-standalone.json"
        )

    def load_provenance(self) -> dict:
        return json.loads(self.provenance_path().read_text(encoding="utf-8"))

    def load_upstream_review(self) -> dict:
        return json.loads(self.upstream_review_path().read_text(encoding="utf-8"))

    def assert_rlm_review_identity(self, review: dict) -> None:
        self.assertEqual(review["schemaVersion"], 1)
        self.assertEqual(review["id"], "2026-08-13-rlm-v1-33-product-update")
        self.assertEqual(review["generatedAt"], "2026-08-14")
        self.assertEqual(
            review["source"],
            {
                "repository": "https://github.com/Dach-Coin/rlm-tools-bsl",
                "tag": "v1.33.0",
                "commit": "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
                "tree": "4b321de0454d4d0998762659891374a3a1326cd0",
                "patches": [],
            },
        )
        self.assertEqual(
            review["toolchain"],
            {
                "repository": "https://github.com/IngvarConsulting/unica-toolchain",
                "releaseTag": "rlm-tools-bsl-v1.33.0-build.2",
                "buildRevision": 2,
            },
        )
        self.assertEqual(
            review["compatibility"],
            {
                "builder": "15",
                "previousBuilder": "14",
                "strategy": "cold-generation-cutover",
                "legacyStateDeleted": False,
                "publicMcpChanged": False,
            },
        )
        self.assertEqual(set(review["tools"]), {"rlm-bsl-mcp", "rlm-bsl-index"})

    def assert_rlm_standalone_review_identity(self, review: dict) -> None:
        self.assertEqual(review["schemaVersion"], 1)
        self.assertEqual(review["id"], "2026-08-14-rlm-v1-33-nuitka-standalone")
        self.assertEqual(review["generatedAt"], "2026-08-14")
        self.assertEqual(
            review["source"],
            {
                "repository": "https://github.com/Dach-Coin/rlm-tools-bsl",
                "tag": "v1.33.0",
                "commit": "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
                "tree": "4b321de0454d4d0998762659891374a3a1326cd0",
                "patches": [],
            },
        )
        self.assertEqual(
            review["toolchain"],
            {
                "repository": "https://github.com/IngvarConsulting/unica-toolchain",
                "releaseTag": "rlm-tools-bsl-v1.33.0-build.3",
                "buildRevision": 3,
                "commit": "b41e85c751bd542265591d3d0fc41dd29c26b8c5",
            },
        )
        self.assertEqual(review["targets"], RLM_STANDALONE_TARGET_EVIDENCE)
        self.assertEqual(
            review["compatibility"],
            {
                "builder": "15",
                "previousBuilder": "14",
                "strategy": "cold-generation-cutover",
                "packaging": "nuitka-standalone-multidist",
                "legacyStateDeleted": False,
                "runtimeLoaderChanged": False,
                "publicMcpChanged": False,
            },
        )

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
        self.assertIn("docs/provenance", path.as_posix())
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

    def test_historical_rlm_build_2_review_is_immutable(self) -> None:
        review = json.loads(
            (
                self.reviews_dir()
                / "2026-08-13-rlm-v1-33-product-update.json"
            ).read_text(encoding="utf-8")
        )

        expected_names = {"rlm-bsl-mcp", "rlm-bsl-index"}
        self.assert_rlm_review_identity(review)
        self.assertEqual(set(review["tools"]), expected_names)
        for name in expected_names:
            self.assertEqual(set(review["tools"][name]), {"assets"})

    def test_rlm_standalone_review_binds_the_published_archive_contract(self) -> None:
        path = self.rlm_standalone_review_path()
        self.assertTrue(path.is_file())
        review = json.loads(path.read_text(encoding="utf-8"))
        self.assert_rlm_standalone_review_identity(review)

        tool_lock = json.loads(
            (self.repo_root() / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        locked = {
            tool["name"]: tool
            for tool in tool_lock["tools"]
            if tool["name"] in {"rlm-bsl-index", "rlm-bsl-mcp"}
        }
        for target, evidence in review["targets"].items():
            for name, tool in locked.items():
                self.assertEqual(tool["assetTag"], review["toolchain"]["releaseTag"])
                asset = tool["assets"][target]
                self.assertEqual(
                    {key: asset[key] for key in ("assetName", "sha256", "size")},
                    evidence["archive"],
                )
                self.assertEqual(
                    asset["archiveBinary"],
                    evidence["entrypoints"][name],
                )

    def test_rlm_standalone_review_rejects_mutated_immutable_metadata(self) -> None:
        review = json.loads(self.rlm_standalone_review_path().read_text(encoding="utf-8"))
        mutations = [
            (("source", "tree"), "0" * 40),
            (("toolchain", "commit"), "0" * 40),
            (("targets", "darwin-arm64", "archive", "sha256"), "0" * 64),
            (("targets", "linux-x64", "payload", "fileCount"), 2),
            (("targets", "win-x64", "builder", "nuitka"), "0.0.0"),
            (("targets", "win-x64", "entrypoints", "rlm-bsl-mcp"), "legacy.exe"),
            (("compatibility", "runtimeLoaderChanged"), True),
        ]
        for path, value in mutations:
            with self.subTest(path=path):
                mutated = copy.deepcopy(review)
                target = mutated
                for key in path[:-1]:
                    target = target[key]
                target[path[-1]] = value
                with self.assertRaises(AssertionError):
                    self.assert_rlm_standalone_review_identity(mutated)

    def test_rlm_review_identity_rejects_mutated_immutable_metadata(self) -> None:
        review = json.loads(
            (
                self.reviews_dir()
                / "2026-08-13-rlm-v1-33-product-update.json"
            ).read_text(encoding="utf-8")
        )
        mutations = [
            (("schemaVersion",), 2),
            (("id",), "different-review"),
            (("source", "repository"), "https://example.invalid/upstream"),
            (("source", "tag"), "v1.33.1"),
            (("source", "commit"), "0" * 40),
            (("source", "tree"), "0" * 40),
            (("source", "patches"), ["local.patch"]),
            (("toolchain", "repository"), "https://example.invalid/toolchain"),
            (("toolchain", "releaseTag"), "rlm-tools-bsl-v1.33.0-build.1"),
            (("toolchain", "buildRevision"), 1),
            (("compatibility", "builder"), "14"),
            (("compatibility", "previousBuilder"), "13"),
            (("compatibility", "strategy"), "migration"),
            (("compatibility", "legacyStateDeleted"), True),
            (("compatibility", "publicMcpChanged"), True),
        ]
        for path, value in mutations:
            with self.subTest(path=path):
                mutated = copy.deepcopy(review)
                target = mutated
                for key in path[:-1]:
                    target = target[key]
                target[path[-1]] = value
                with self.assertRaises(AssertionError):
                    self.assert_rlm_review_identity(mutated)

    def test_bsl_analyzer_contract_is_v0_2_67(self) -> None:
        tool_lock = json.loads(
            (self.repo_root() / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        locked_tools = {tool["name"]: tool for tool in tool_lock["tools"]}

        analyzer = locked_tools["bsl-analyzer"]
        self.assertEqual(analyzer["version"], "0.2.67")
        self.assertEqual(analyzer["sourceTag"], "v0.2.67")
        self.assertEqual(
            analyzer["sourceCommit"],
            "9a92766691bbd0191a5ff02c34fa9058e4570b85",
        )
        self.assertEqual(analyzer["assetTag"], "bsl-analyzer-v0.2.67-build.1")
        self.assertEqual(
            analyzer["assets"],
            {
                "darwin-arm64": {
                    "assetName": "bsl-analyzer-darwin-arm64",
                    "sha256": "d18c3b79d017d60f229faf4e427bcefc0a9da59a93b57acbb867b064c52926bd",
                },
                "linux-x64": {
                    "assetName": "bsl-analyzer-linux-x64",
                    "sha256": "c476c10fcdfa6eb7d310e83d0e69b02a27f9afeec0d394681feadb889de97301",
                },
                "win-x64": {
                    "assetName": "bsl-analyzer-win-x64.exe",
                    "sha256": "a54d883bcb7ed0e0039953fb4d5cd7c2efbf30155de9951952f1a4060776eb3e",
                },
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

        self.assertEqual(backlog["generatedAt"], "2026-08-12")
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
