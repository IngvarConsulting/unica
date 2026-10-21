from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


def load_classifier_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "classify-workflow-changes.py"
    spec = importlib.util.spec_from_file_location("classify_workflow_changes", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


OUTPUT_NAMES = (
    "rust_changed",
    "platform_changed",
    "toolchain_changed",
    "search_integration_changed",
    "package_changed",
    "plugin_content_changed",
    "ci_changed",
    "release_required",
    "assessment_required",
)


class GitEventClassificationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_classifier_module()

    def git(self, repo: Path, *arguments: str) -> str:
        return subprocess.run(["git", *arguments], cwd=repo, check=True, capture_output=True, text=True).stdout.strip()

    def repository(self) -> tuple[Path, str]:
        temporary = tempfile.TemporaryDirectory(prefix="ci-event-")
        self.addCleanup(temporary.cleanup)
        repo = Path(temporary.name)
        self.git(repo, "init", "-b", "main")
        self.git(repo, "config", "user.name", "CI test")
        self.git(repo, "config", "user.email", "ci@example.invalid")
        self.git(repo, "config", "commit.gpgsign", "false")
        self.git(repo, "config", "core.filemode", "true")
        (repo / "README.md").write_text("Before\n")
        (repo / "Cargo.toml").write_text("# manifest fixture\n")
        return repo, self.commit(repo)

    def commit(self, repo: Path) -> str:
        self.git(repo, "add", "--all")
        self.git(repo, "commit", "-m", "fixture")
        return self.git(repo, "rev-parse", "HEAD")

    def queue(self, repo: Path, base: str, head: str):
        return self.module.classify_event(repo, "merge_group", "refs/heads/gh-readonly-queue/main/pr-1", head,
            {"merge_group": {"base_sha": base, "head_sha": head}})

    def assert_full(self, scope) -> None:
        self.assertFalse(scope.docs_only)
        self.assertTrue(all(scope.classification.as_dict().values()), scope)

    def invoke_workflow(self, repo: Path, event: dict, head: str):
        payload = repo / "event.json"
        payload.write_text(json.dumps(event))
        script = Path(__file__).resolve().parents[2] / "scripts/ci/classify-workflow-changes.py"
        environment = {**os.environ, "GITHUB_EVENT_PATH": str(payload),
            "GITHUB_EVENT_NAME": "merge_group", "GITHUB_REF": "refs/heads/gh-readonly-queue/main/pr-1",
            "GITHUB_SHA": head, "GATE_PROFILE": "queue", "GITHUB_REPOSITORY": "owner/repo",
            "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "2"}
        return subprocess.run([sys.executable, str(script), "--github-event", "--runner-matrix",
            '["ubuntu-latest", "macos-14"]', "--line", "main", "--results", str(repo / "routing")],
            cwd=repo, env=environment, capture_output=True, text=True)

    def test_workflow_cli_routes_docs_and_signs_skip_without_fabricating_results(self) -> None:
        repo, base = self.repository()
        (repo / "README.md").write_text("After\n")
        head = self.commit(repo)
        result = self.invoke_workflow(repo, {"merge_group": {"base_sha": base, "head_sha": head}}, head)
        self.assertEqual(0, result.returncode, result.stderr)
        outputs = dict(line.split("=", 1) for line in result.stdout.splitlines())
        self.assertEqual("true", outputs["docs_only"])
        self.assertEqual("false", outputs["rust_required"])
        self.assertEqual("false", outputs["cache_required"])
        self.assertTrue(all(outputs[name] == "false" for name in OUTPUT_NAMES))
        self.assertFalse((repo / "routing/run.json").exists())
        self.assertEqual(2, len(list((repo / "routing").rglob("*.*"))))
        for runner in ("ubuntu-latest", "macos-14"):
            marker = json.loads((repo / f"routing/results-rust-{runner}/run.json").read_text())
            self.assertEqual(("rust", "queue", "main", runner, head),
                tuple(marker[field] for field in ("ecosystem", "profile", "ref", "runner", "sha")))
            self.assertEqual("https://github.com/owner/repo/actions/runs/123", marker["run_url"])
            self.assertEqual("2", marker["run_attempt"])
            self.assertEqual({"reason": "docs_only", "base_sha": base, "head_sha": head}, marker["scope_skip"])
        # Exercise the artifact handoff through the skipped inventory aggregate.
        # No previous inventory means no test cases.
        collector_path = Path(__file__).resolve().parents[2] / "scripts/ci/collect-results.py"
        spec = importlib.util.spec_from_file_location("collect_event_results", collector_path)
        collector = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(collector)
        jobs = repo / "jobs.json"
        jobs.write_text(json.dumps({"jobs": [{"name": "Rust tests (complete inventory)", "conclusion": "skipped"}]}))
        collector.collect(repo / "routing", repo / "report", jobs, "", "")
        report = json.loads((repo / "report/main/run.json").read_text())
        self.assertEqual(head, report["sha"])
        self.assertEqual({"ubuntu-latest", "macos-14"}, {item["runner"] for item in report["routing_skips"]})
        self.assertTrue(all(item["inventory_count"] == 0 for item in report["routing_skips"]))
        self.assertEqual([], list((repo / "report").rglob("*-result.json")))

    def test_workflow_cli_unknown_diff_runs_full_queue_and_emits_no_skip_artifact(self) -> None:
        repo, base = self.repository()
        (repo / "README.md").write_text("After\n")
        head = self.commit(repo)
        result = self.invoke_workflow(repo, {"merge_group": {"head_sha": head}}, head)
        self.assertEqual(0, result.returncode, result.stderr)
        outputs = dict(line.split("=", 1) for line in result.stdout.splitlines())
        self.assertEqual("false", outputs["docs_only"])
        self.assertEqual("true", outputs["rust_required"])
        self.assertTrue(all(outputs[name] == "true" for name in OUTPUT_NAMES))
        self.assertFalse((repo / "routing").exists())

    def test_queue_known_prose_only_has_immutable_scope(self) -> None:
        repo, base = self.repository()
        (repo / "README.md").write_text("After\n")
        (repo / "arch/rules").mkdir(parents=True)
        (repo / "arch/rules/example.md").write_text("Editorial rule\n")
        head = self.commit(repo)
        scope = self.queue(repo, base, head)
        self.assertTrue(scope.docs_only)
        self.assertFalse(any(scope.classification.as_dict().values()))
        self.assertEqual((base, head), (scope.base_sha, scope.head_sha))

    def test_entire_group_including_non_prose_inputs_keeps_full_queue(self) -> None:
        for path in (
            "crates/example/src/lib.rs", "scripts/ci/helper.py", "tests/fixtures/README.md",
            "plugins/unica/skills/test/SKILL.md", "plugins/unica/references/specs/test.md",
            "docs/tool-surface.md", "docs/provenance/skill-upstreams.json",
            "docs/pages/site.js", "docs/unknown.md", "Cargo.lock", ".github/workflows/test.yml",
        ):
            with self.subTest(path=path):
                repo, base = self.repository()
                (repo / "README.md").write_text("Docs commit\n")
                self.commit(repo)
                target = repo / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text("Another PR in this group\n")
                self.assert_full(self.queue(repo, base, self.commit(repo)))

    def test_renames_deletions_symlinks_and_executable_modes_never_skip_rust(self) -> None:
        for kind in ("rename", "delete", "symlink", "mode"):
            with self.subTest(kind=kind):
                repo, base = self.repository()
                readme = repo / "README.md"
                if kind == "rename":
                    readme.rename(repo / "CONTRIBUTING.md")
                elif kind == "mode":
                    readme.chmod(0o755)
                else:
                    readme.unlink()
                    if kind == "symlink":
                        readme.symlink_to("Cargo.toml")
                incoming = self.commit(repo)
                self.assert_full(self.queue(repo, base, incoming))
                self.git(repo, "checkout", "--detach", base)
                self.git(repo, "merge", "--no-ff", incoming, "-m", "event merge")
                merged = self.git(repo, "rev-parse", "HEAD")
                event = {"pull_request": {"base": {"sha": base}, "head": {"sha": incoming}, "labels": []}}
                self.assert_full(self.module.classify_event(repo, "pull_request", "refs/pull/1/merge", merged, event))

    def test_bad_or_stale_queue_identity_falls_back_to_all_contours(self) -> None:
        repo, base = self.repository()
        (repo / "README.md").write_text("After\n")
        head = self.commit(repo)
        for event in ({}, {"merge_group": {}},
            {"merge_group": {"base_sha": "0" * 40, "head_sha": head}},
            {"merge_group": {"base_sha": base, "head_sha": base}},
        ):
            with self.subTest(event=event):
                self.assert_full(self.module.classify_event(repo, "merge_group", "refs/heads/queue", head, event))
        self.assert_full(self.queue(repo, head, head))
        self.git(repo, "checkout", "--detach", base)
        self.assert_full(self.queue(repo, base, head))

    def test_pr_uses_event_merge_parents_not_a_moving_base_branch(self) -> None:
        repo, base = self.repository()
        self.git(repo, "checkout", "-b", "topic")
        (repo / "README.md").write_text("PR prose\n")
        incoming = self.commit(repo)
        self.git(repo, "checkout", "main")
        self.git(repo, "merge", "--no-ff", "topic", "-m", "event merge")
        merged = self.git(repo, "rev-parse", "HEAD")
        event = {"pull_request": {"base": {"sha": base}, "head": {"sha": incoming}, "labels": []}}
        # Moving main does not alter the event's verified merge tree.
        (repo / "Cargo.toml").write_text("# later code change\n")
        moved = self.commit(repo)
        self.git(repo, "checkout", "--detach", merged)
        scope = self.module.classify_event(repo, "pull_request", "refs/pull/1/merge", merged, event)
        self.assertTrue(scope.docs_only)
        event["pull_request"]["base"]["sha"] = moved
        self.assert_full(self.module.classify_event(repo, "pull_request", "refs/pull/1/merge", merged, event))
        event["pull_request"]["base"]["sha"] = base
        event["pull_request"]["labels"] = [{"name": "ci:full"}]
        self.assert_full(self.module.classify_event(repo, "pull_request", "refs/pull/1/merge", merged, event))

    def test_pr_keeps_existing_non_rust_contours_but_the_same_code_queue_is_full(self) -> None:
        repo, base = self.repository()
        self.git(repo, "checkout", "-b", "topic")
        (repo / "README.md").write_text("Docs plus a plugin instruction\n")
        instruction = repo / "plugins/unica/skills/example/SKILL.md"
        instruction.parent.mkdir(parents=True)
        instruction.write_text("A shipped instruction is not editorial prose\n")
        incoming = self.commit(repo)
        self.git(repo, "checkout", "main")
        self.git(repo, "merge", "--no-ff", "topic", "-m", "event merge")
        head = self.git(repo, "rev-parse", "HEAD")
        event = {"pull_request": {"base": {"sha": base}, "head": {"sha": incoming}, "labels": []}}
        scope = self.module.classify_event(repo, "pull_request", "refs/pull/1/merge", head, event)
        self.assertFalse(scope.docs_only)
        self.assertTrue(scope.classification.plugin_content_changed)
        self.assertFalse(scope.classification.rust_changed)
        self.assertFalse(scope.classification.ci_changed)
        self.assert_full(self.queue(repo, base, head))

    def test_tag_manual_and_release_branch_do_not_use_documentation_exception(self) -> None:
        repo, base = self.repository()
        (repo / "README.md").write_text("After\n")
        head = self.commit(repo)
        for event, ref in (("push", "refs/tags/v1.0.0"), ("push", "refs/heads/release-v1.0"), ("workflow_dispatch", "refs/heads/main")):
            with self.subTest(event=event, ref=ref):
                self.assert_full(self.module.classify_event(repo, event, ref, head, {"before": base, "after": head}))
        scope = self.module.classify_event(repo, "push", "refs/heads/main", head, {"before": base, "after": head})
        self.assertFalse(scope.docs_only)
        self.assertFalse(any(scope.classification.as_dict().values()))

    def test_raw_diff_does_not_split_paths_on_newlines_or_normalize_unsafe_names(self) -> None:
        raw = ":100644 100644 aaaaaaa bbbbbbb M\0arch/a\nb.md\0"
        changes = self.module.parse_raw_diff(raw)
        self.assertEqual(["arch/a\nb.md"], [change.path for change in changes])
        for path in ("./README.md", "../README.md", "/README.md", "arch/../README.md", "arch\\a.md", "arch/a\nb.md"):
            with self.subTest(path=path):
                self.assertFalse(self.module.is_prose_path(path))


class ClassifyWorkflowChangesTests(unittest.TestCase):
    def assert_classification(self, paths: list[str], **expected: bool) -> None:
        module = load_classifier_module()
        classification = module.classify_paths(paths)
        actual = classification.as_dict()
        self.assertEqual(set(OUTPUT_NAMES), set(actual))
        self.assertEqual({name: expected.get(name, False) for name in OUTPUT_NAMES}, actual)

    def test_skill_only_change_stays_in_source_contour(self) -> None:
        self.assert_classification(
            ["plugins/unica/skills/meta-add/SKILL.md"],
            plugin_content_changed=True,
        )

    def test_maintainer_provenance_is_not_plugin_content(self) -> None:
        """The donor index and its review records ship with nothing.

        They live outside `plugins/unica/`, so a change to them must not claim
        the plugin content contour. `test-python` runs unconditionally and
        still covers the attribution and provenance contracts.
        """
        self.assert_classification(
            [
                "docs/provenance/skill-upstreams.json",
                "docs/provenance/reviews/historical-review.json",
            ],
        )

    def test_platform_independent_domain_or_application_rust_uses_primary_rust_contour(self) -> None:
        for path in (
            "crates/unica-coder/src/domain/cache.rs",
            "crates/unica-coder/src/application/metadata.rs",
            "vendor/ureq/src/response.rs",
            "vendor/ureq/src/completed_response_recycling.rs",
        ):
            with self.subTest(path=path):
                self.assert_classification([path], rust_changed=True, release_required=True)

    def test_platform_facade_and_platform_tests_require_platform_matrix(self) -> None:
        for path in (
            "crates/unica-coder/src/infrastructure/platform/filesystem.rs",
            "crates/unica-coder/src/infrastructure/platform_xml_owner.rs",
            "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs",
            "crates/unica-coder/src/infrastructure/source_roots.rs",
            "crates/unica-coder/src/infrastructure/path_policy.rs",
            "crates/unica-coder/src/infrastructure/support_state.rs",
            "crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs",
            "crates/unica-bootstrap/src/platform/mod.rs",
            "crates/unica-coder/tests/platform/new_contract.rs",
            "crates/unica-coder/tests/platform_external_init.rs",
            "crates/unica-coder/src/infrastructure/platform/unknown.future",
        ):
            with self.subTest(path=path):
                search_integration_changed = path in {
                    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs",
                    "crates/unica-coder/src/infrastructure/source_roots.rs",
                }
                self.assert_classification(
                    [path],
                    rust_changed=True,
                    platform_changed=True,
                    search_integration_changed=search_integration_changed,
                    release_required=True,
                    assessment_required=search_integration_changed,
                )

    def test_cargo_and_toolchain_changes_require_full_rust_and_package_contours(self) -> None:
        # `.config/nextest.toml` меняет, как гоняется каждая цель на каждом
        # раннере, — тот же класс, что смена toolchain.
        for path in (
            "Cargo.toml",
            "Cargo.lock",
            "crates/unica-coder/Cargo.toml",
            "rust-toolchain.toml",
            ".config/nextest.toml",
            ".config/python-sizes.toml",
            "vendor/ureq/Cargo.toml",
            "vendor/ureq/Cargo.lock",
            "vendor/ureq/.config/nextest.toml",
        ):
            with self.subTest(path=path):
                self.assert_classification(
                    [path],
                    rust_changed=True,
                    toolchain_changed=True,
                    search_integration_changed=True,
                    package_changed=True,
                    release_required=True,
                    assessment_required=True,
                )

    def test_vendor_routing_is_scoped_to_the_patched_dependency_build_inputs(self) -> None:
        for path in ("vendor/other/src/response.rs", "vendor/ureq/README.md", "vendor/ureq/Cargo.toml.orig"):
            with self.subTest(path=path):
                self.assert_classification([path])

    def test_search_and_rlm_mechanism_changes_route_the_long_integration_test(self) -> None:
        for path in (
            "crates/unica-coder/src/application/code_intelligence.rs",
            "crates/unica-coder/src/application/mod.rs",
            "crates/unica-coder/src/application/operational_config.rs",
            "crates/unica-coder/src/application/ports.rs",
            "crates/unica-coder/src/application/source_navigation.rs",
            "crates/unica-coder/src/application/tool_contracts.rs",
            "crates/unica-coder/src/domain/code_intelligence.rs",
            "crates/unica-coder/src/domain/operational_config.rs",
            "crates/unica-coder/src/domain/source_location.rs",
            "crates/unica-coder/src/infrastructure/application_ports.rs",
            "crates/unica-coder/src/infrastructure/code_intelligence.rs",
            "crates/unica-coder/src/infrastructure/internal_adapters.rs",
            "crates/unica-coder/src/infrastructure/operational_config.rs",
            "crates/unica-coder/src/infrastructure/platform/process.rs",
            "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs",
            "crates/unica-coder/src/infrastructure/rlm_navigation.rs",
            "crates/unica-coder/src/infrastructure/source_roots.rs",
            "crates/unica-coder/src/infrastructure/workspace_index.rs",
            "crates/unica-coder/src/infrastructure/workspace_services.rs",
            "crates/unica-coder/src/interfaces/mcp.rs",
        ):
            with self.subTest(path=path):
                platform_changed = (
                    "/platform/" in path
                    or "/tests/platform/" in path
                    or path
                    in {
                        "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs",
                        "crates/unica-coder/src/infrastructure/source_roots.rs",
                    }
                )
                self.assert_classification(
                    [path],
                    rust_changed=True,
                    platform_changed=platform_changed,
                    search_integration_changed=True,
                    release_required=True,
                    assessment_required=True,
                )

    def test_extracted_analysis_owners_keep_their_real_search_consumers(self) -> None:
        for path in (
            "crates/unica-code-intelligence/src/coordinator.rs",
            "crates/unica-code-intelligence/src/model.rs",
            "crates/unica-configuration/src/operational.rs",
            "crates/unica-source-contract/src/location.rs",
            "crates/unica-source-contract/src/workspace.rs",
        ):
            with self.subTest(path=path):
                self.assert_classification(
                    [path], rust_changed=True, search_integration_changed=True,
                    release_required=True, assessment_required=True,
                )
        self.assert_classification(["crates/unica-code-intelligence/README.md"])
        self.assert_classification(
            ["crates/unica-code-intelligence-extra/src/model.rs"],
            rust_changed=True, release_required=True,
        )

    def test_package_contract_changes_require_package_contour(self) -> None:
        for path in (
            "plugins/unica/.codex-plugin/plugin.json",
            "plugins/unica/.claude-plugin/plugin.json",
            "plugins/unica/.zcode-plugin/plugin.json",
            "plugins/unica/.mcp.json",
            "plugins/unica/third-party/tools.lock.json",
            "scripts/ci/package-unica-runtime.py",
            "scripts/ci/stage-unica-assessment-engine.py",
        ):
            with self.subTest(path=path):
                self.assert_classification(
                    [path],
                    package_changed=True,
                    plugin_content_changed=path.startswith("plugins/unica/"),
                    release_required=True,
                    assessment_required=path in {
                        "plugins/unica/third-party/tools.lock.json",
                        "scripts/ci/package-unica-runtime.py",
                        "scripts/ci/stage-unica-assessment-engine.py",
                    },
                )

    def test_classifier_workflow_and_platform_guard_changes_fail_closed(self) -> None:
        cases = {
            ".github/workflows/unica-plugin-release.yml": {
                "ci_changed": True,
                "assessment_required": True,
            },
            "scripts/ci/classify-workflow-changes.py": {
                "ci_changed": True,
                "assessment_required": True,
            },
            "tests/ci/test_classify_workflow_changes.py": {
                "ci_changed": True,
                "assessment_required": True,
            },
            "scripts/ci/check-rust-platform-boundary.py": {
                "rust_changed": True,
                "platform_changed": True,
                "ci_changed": True,
                "release_required": True,
            },
            "tests/ci/test_rust_platform_boundary.py": {
                "rust_changed": True,
                "platform_changed": True,
                "ci_changed": True,
                "release_required": True,
            },
        }
        for path, expected in cases.items():
            with self.subTest(path=path):
                self.assert_classification([path], **expected)

    def test_release_matrix_reports_standalone_runtime_size_evidence(self) -> None:
        workflow = (
            Path(__file__).resolve().parents[2]
            / ".github"
            / "workflows"
            / "unica-plugin-release.yml"
        ).read_text(encoding="utf-8")

        self.assertIn("Report standalone runtime size evidence", workflow)
        for label in (
            "RLM source archive bytes",
            "RLM extracted payload bytes",
            "Final runtime archive bytes",
            "Runtime file count",
        ):
            with self.subTest(label=label):
                self.assertIn(label, workflow)
        self.assertIn('manifest["runtimeFiles"]', workflow)
        self.assertIn('archive.extractfile("manifest.json")', workflow)

    def test_local_installer_change_requires_ci_contour(self) -> None:
        self.assert_classification(
            ["scripts/dev/install-local-unica.sh"],
            ci_changed=True,
        )

    def test_archive_prepared_and_shadow_changes_cannot_take_the_prose_or_source_route(self) -> None:
        module = load_classifier_module()
        for path in (
            "scripts/ci/ci_rust_lanes.py", "scripts/ci/prepared_binary.py", "scripts/ci/ci_shadow_selector.py",
            "tests/ci/test_ci_rust_lanes.py", "tests/ci/test_ci_prepared_binary.py", "tests/ci/test_ci_shadow_selector.py",
        ):
            with self.subTest(path=path):
                classification = module.classify_paths([path])
                self.assertTrue(classification.ci_changed)
                self.assertFalse(module.is_prose_path(path))
                flags = classification._asdict()
                self.assertTrue(module.rust_route("pull_request", "refs/pull/1/merge", flags, docs_only=False).required)
                self.assertTrue(module.rust_route("merge_group", "refs/heads/queue", flags, docs_only=False).required)
        prose = module.classify_paths(["CONTRIBUTING.md"])
        self.assertFalse(prose.ci_changed)
        self.assertFalse(module.rust_route("pull_request", "refs/pull/1/merge", prose._asdict(), docs_only=True).required)

    def test_the_test_seam_and_its_guard_require_the_ci_contour(self) -> None:
        """Шов решает, что гоняет каждая джоба: его правка — правка конвейера.

        Иначе правка одного `run-tests.py` ехала бы контуром исходников и
        могла бы тихо сузить прогон, который сама и должна была запустить.
        """
        for path in (
            "scripts/ci/run-tests.py",
            "scripts/ci/run-unittest.py",
            "scripts/ci/allure_results.py",
            "scripts/ci/collect-results.py",
            "tests/ci/test_run_tests.py",
            "tests/ci/test_gate_profiles.py",
            "scripts/ci/nightly-lines.py",
            "scripts/ci/resolve-line.py",
            "tests/ci/test_nightly_lines.py",
            "tests/ci/test_resolve_line.py",
            "scripts/ci/size-filters.py",
            "tests/ci/test_size_guard.py",
            "tests/ci/test_python_sizes.py",
            "tests/ci/test_research_policy.py",
            "tests/ci/test_allure_results.py",
            "tests/ci/test_collect_results.py",
        ):
            with self.subTest(path=path):
                self.assert_classification([path], ci_changed=True)

    def test_p0_release_proof_changes_route_package_and_assessment_contours(self) -> None:
        self.assert_classification(
            ["scripts/ci/release-proof.py"],
            package_changed=True,
            release_required=True,
            assessment_required=True,
        )

    def test_mixed_changes_union_their_contours(self) -> None:
        self.assert_classification(
            [
                "plugins/unica/skills/meta-add/SKILL.md",
                "crates/unica-coder/src/infrastructure/platform/process.rs",
                "plugins/unica/.codex-plugin/plugin.json",
            ],
            rust_changed=True,
            platform_changed=True,
            search_integration_changed=True,
            package_changed=True,
            plugin_content_changed=True,
            release_required=True,
            assessment_required=True,
        )

    def test_release_assessment_routes_only_affected_mechanism(self) -> None:
        """The hour-long BSP assessment is not the price of any Rust change.

        The contour above proves the search mechanism routes it. What this one
        states is the boundary: unrelated Rust stays out, and the packaged
        runtime the assessment unpacks routes it without belonging to the
        search contour at all.
        """
        self.assert_classification(
            ["crates/unica-coder/src/domain/cache.rs"],
            rust_changed=True,
            release_required=True,
        )

        self.assert_classification(
            ["crates/unica-coder/src/infrastructure/plugin_runtime.rs"],
            rust_changed=True,
            release_required=True,
            assessment_required=True,
        )

        self.assert_classification(
            ["scripts/ci/release-assessment.py"],
            package_changed=True,
            release_required=True,
            assessment_required=True,
        )

    def test_p0_evidence_producers_route_package_and_proof_contours(self) -> None:
        for path in (
            "scripts/ci/package-unica-plugin.py",
            "scripts/ci/probe-unica-wire.py",
            "scripts/ci/verify-release-assets.py",
        ):
            with self.subTest(path=path):
                self.assert_classification(
                    [path],
                    package_changed=True,
                    release_required=True,
                    assessment_required=True,
                )

    def test_every_assessment_path_also_claims_a_release_or_ci_contour(self) -> None:
        """`evaluate-ci-gate.py` reads a lone assessment contour as a contradiction.

        The assessment job hangs off the package pipeline, so a path that
        routes it while claiming neither contour describes a run that cannot
        happen — and the gate fails the whole workflow rather than the one
        job. Deriving the contour from the search paths hides this, so the
        explicit list is what needs stating.
        """
        module = load_classifier_module()

        offenders = []
        for path in sorted(module.ASSESSMENT_PATHS):
            values = module.classify_paths([path]).as_dict()
            if not (values["release_required"] or values["ci_changed"]):
                offenders.append(path)

        self.assertEqual([], offenders)

    def test_release_assessment_affected_contour_is_closed(self) -> None:
        module = load_classifier_module()

        for path in sorted(module.ASSESSMENT_PATHS):
            with self.subTest(affected=path):
                self.assertTrue(module.classify_paths([path]).assessment_required)
        for path in (
            "README.md",
            "plugins/unica/skills/meta-add/SKILL.md",
            "crates/unica-coder/src/domain/cache.rs",
            "docs/provenance/skill-upstreams.json",
        ):
            with self.subTest(unaffected=path):
                self.assertFalse(module.classify_paths([path]).assessment_required)
        self.assertTrue(module.classify_paths([], force_full=True).assessment_required)

    def test_forced_full_contour_enables_every_output(self) -> None:
        module = load_classifier_module()

        classification = module.classify_paths([], force_full=True)

        self.assertEqual({name: True for name in OUTPUT_NAMES}, classification.as_dict())

    def test_cli_prints_github_outputs_from_stdin_paths(self) -> None:
        module = load_classifier_module()
        with tempfile.TemporaryFile("w+", encoding="utf-8") as stdin:
            stdin.write("plugins/unica/skills/meta-add/SKILL.md\ncrates/unica-bootstrap/src/main.rs\n")
            stdin.seek(0)

            output = module.classify_stdin(stdin)

        self.assertEqual(
            {
                "rust_changed=true",
                "platform_changed=false",
                "toolchain_changed=false",
                "search_integration_changed=false",
                "package_changed=false",
                "plugin_content_changed=true",
                "ci_changed=false",
                "release_required=true",
                "assessment_required=false",
            },
            set(output.splitlines()),
        )


if __name__ == "__main__":
    unittest.main()
