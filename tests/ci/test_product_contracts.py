from __future__ import annotations

import importlib.util
import json
import re
import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path
from unittest.mock import patch


def load_contract_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "check-tool-contracts.py"
    spec = importlib.util.spec_from_file_location("check_tool_contracts", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ProductContractTests(unittest.TestCase):
    def test_native_validators_do_not_expose_internal_local_owner_only_switch(
        self,
    ) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        rust_root = repo_root / "crates" / "unica-coder" / "src"
        offenders = []
        for path in sorted(rust_root.rglob("*.rs")):
            text = path.read_text(encoding="utf-8")
            for marker in ("InternalLocalOwnerOnly", "internalLocalOwnerOnly"):
                if marker in text:
                    offenders.append(
                        f"{path.relative_to(repo_root).as_posix()}: {marker}"
                    )
        self.assertEqual(offenders, [])

    def test_v8_runner_partial_load_list_requires_bom_crlf_and_cyrillic_path(self) -> None:
        module = load_contract_module()
        expected_path = str(
            Path("Catalogs.Товары") / "Ext" / "ObjectModule.bsl"
        )
        payload = b"\xef\xbb\xbf" + expected_path.encode("utf-8") + b"\r\n"

        self.assertEqual(
            module.validate_v8_runner_partial_load_list(payload, expected_path),
            [],
        )
        self.assertIn(
            "UTF-8 BOM",
            "\n".join(
                module.validate_v8_runner_partial_load_list(
                    payload.removeprefix(b"\xef\xbb\xbf"),
                    expected_path,
                )
            ),
        )
        self.assertIn(
            "CRLF",
            "\n".join(
                module.validate_v8_runner_partial_load_list(
                    b"\xef\xbb\xbf" + expected_path.encode("utf-8") + b"\n",
                    expected_path,
                )
            ),
        )

    def test_v8_runner_partial_load_smoke_rejects_missing_binary(self) -> None:
        module = load_contract_module()

        errors = module.check_v8_runner_partial_load_contract(
            Path("/missing/v8-runner"),
            "linux-x64",
        )

        self.assertEqual(
            errors,
            ["v8-runner partial-load contract: binary not found: /missing/v8-runner"],
        )

    def test_v8_runner_bounded_external_epf_result_accepts_exit_seven_artifacts(
        self,
    ) -> None:
        module = load_contract_module()
        validator = getattr(
            module,
            "validate_v8_runner_bounded_external_epf_result",
            None,
        )
        self.assertIsNotNone(validator)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            execute = root / "processor.epf"
            output = root / "platform.log"
            stderr_output = root / "client.stderr.log"
            execute.write_bytes(b"epf")
            output.write_text("bounded-platform-out\n", encoding="utf-8")
            stderr_output.write_text("bounded-client-stderr\n", encoding="utf-8")
            envelope = {
                "data": {
                    "external_epf_wait": {
                        "pid": 123,
                        "execute_path": str(execute),
                        "exit_code": 7,
                        "timed_out": False,
                        "output_path": str(output),
                        "stderr_path": str(stderr_output),
                    }
                }
            }

            self.assertEqual(
                validator(
                    envelope,
                    execute,
                    output,
                    stderr_output,
                    "bounded-platform-out",
                    "bounded-client-stderr",
                ),
                [],
            )

    def test_v8_runner_bounded_external_epf_result_rejects_broken_wait_contract(
        self,
    ) -> None:
        module = load_contract_module()
        validator = getattr(
            module,
            "validate_v8_runner_bounded_external_epf_result",
            None,
        )
        self.assertIsNotNone(validator)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            execute = root / "processor.epf"
            output = root / "platform.log"
            stderr_output = root / "client.stderr.log"
            execute.write_bytes(b"epf")
            output.write_text("bounded-platform-out\n", encoding="utf-8")
            stderr_output.write_text("bounded-client-stderr\n", encoding="utf-8")
            envelope = {
                "data": {
                    "external_epf_wait": {
                        "pid": 123,
                        "execute_path": str(execute),
                        "exit_code": 7,
                        "timed_out": False,
                        "output_path": str(output),
                        "stderr_path": str(stderr_output),
                    }
                }
            }
            mutations = [
                ("pid", 0, "pid"),
                ("execute_path", str(root / "other.epf"), "execute_path"),
                ("exit_code", 0, "exit_code"),
                ("timed_out", True, "timed_out"),
                ("output_path", str(root / "other.log"), "output_path"),
                ("stderr_path", str(root / "other.stderr.log"), "stderr_path"),
            ]

            for field, value, expected_error in mutations:
                with self.subTest(field=field):
                    broken = json.loads(json.dumps(envelope))
                    broken["data"]["external_epf_wait"][field] = value
                    errors = validator(
                        broken,
                        execute,
                        output,
                        stderr_output,
                        "bounded-platform-out",
                        "bounded-client-stderr",
                    )
                    self.assertTrue(
                        any(expected_error in error for error in errors),
                        errors,
                    )

            stderr_output.write_text("unexpected stderr\n", encoding="utf-8")
            errors = validator(
                envelope,
                execute,
                output,
                stderr_output,
                "bounded-platform-out",
                "bounded-client-stderr",
            )
            self.assertTrue(
                any("stderr artifact" in error for error in errors),
                errors,
            )

            output.write_text(
                "bounded-platform-out\nbounded-client-stderr\n",
                encoding="utf-8",
            )
            stderr_output.write_text(
                "bounded-client-stderr\nbounded-platform-out\n",
                encoding="utf-8",
            )
            errors = validator(
                envelope,
                execute,
                output,
                stderr_output,
                "bounded-platform-out",
                "bounded-client-stderr",
            )
            self.assertTrue(
                any("platform /Out artifact" in error for error in errors),
                errors,
            )
            self.assertTrue(
                any("stderr artifact" in error for error in errors),
                errors,
            )

    def test_targeted_tool_contracts_run_both_v8_runner_behavioral_smokes(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            runner = tools_dir / "v8-runner"
            runner.write_bytes(b"runner")
            with (
                patch.object(module, "TOOL_HELP_CHECKS", []),
                patch.object(
                    module,
                    "check_v8_runner_partial_load_contract",
                    return_value=["behavioral failure"],
                ) as behavioral_check,
                patch.object(
                    module,
                    "check_v8_runner_bounded_external_epf_contract",
                    return_value=["bounded failure"],
                ) as bounded_check,
            ):
                errors = module.check_tool_contracts(tools_dir, "linux-x64")

        self.assertEqual(errors, ["behavioral failure", "bounded failure"])
        behavioral_check.assert_called_once_with(runner.resolve(), "linux-x64")
        bounded_check.assert_called_once_with(runner.resolve(), "linux-x64")

    BSL_ANALYZER_HELP = (
        "#!/usr/bin/env sh\n"
        "case \"$*\" in\n"
        "  'analyze --help') printf '%s\\n' '--source-dir --format jsonl' ;;\n"
        "  'mcp serve --help') printf '%s\\n' '--profile --source-dir --mode stdio' ;;\n"
        "  *) exit 1 ;;\n"
        "esac\n"
    )

    def test_local_1ci_corpus_is_ignored_and_agent_discoverable(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        ignore = (repo_root / ".gitignore").read_text(encoding="utf-8")
        agents = (repo_root / "AGENTS.md").read_text(encoding="utf-8")
        package_script = (repo_root / "scripts/ci/package-unica-plugin.py").read_text(
            encoding="utf-8"
        )

        self.assertIn("docs-local/", ignore.splitlines())
        self.assertIn("docs-local/1ci/8.3.27/en/", agents)
        self.assertIn("python3.12 scripts/dev/download-1ci-guides.py", agents)
        self.assertNotIn("docs-local", package_script)

    def test_marketplace_card_uses_unica_product_legal_links(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        plugin = json.loads(
            (repo_root / "plugins/unica/.codex-plugin/plugin.json").read_text(
                encoding="utf-8"
            )
        )

        self.assertEqual(
            plugin["interface"]["websiteURL"],
            "https://ingvar.pro/products/unica/en",
        )
        self.assertEqual(
            plugin["interface"]["privacyPolicyURL"],
            "https://ingvar.pro/products/unica/privacy/en",
        )
        self.assertEqual(
            plugin["interface"]["termsOfServiceURL"],
            "https://ingvar.pro/products/unica/terms/en",
        )

    def test_release_runbook_is_discoverable_and_names_the_tag_target(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        runbook = repo_root / "docs/release-runbook.md"
        agents = (repo_root / "AGENTS.md").read_text(encoding="utf-8")
        skill = (repo_root / ".claude/skills/release/SKILL.md").read_text(encoding="utf-8")

        self.assertTrue(runbook.is_file())
        # An agent asked to release has to reach the runbook from the entry point
        # rather than reconstruct the order from the workflows.
        self.assertIn("docs/release-runbook.md", agents)
        self.assertIn("docs/release-runbook.md", skill)

        text = runbook.read_text(encoding="utf-8")
        for value in (
            "staging merge commit",
            "bump-version.py",
            "check-version-contract.py",
            "publish-unica-marketplace.yml",
            # A release that fails part-way has to have a documented way out.
            "One-way doors",
            "never reuse a version number",
            "Rolling back a live release",
            "Release Warden",
        ):
            with self.subTest(value=value):
                self.assertIn(value, text)

    def test_warden_cannot_publish_without_the_human_tag(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        warden = (repo_root / "scripts/ci/release-warden.py").read_text(encoding="utf-8")
        workflow = (repo_root / ".github/workflows/release-warden.yml").read_text(
            encoding="utf-8"
        )

        # The marketplace default branch has no protection rules, so the
        # greenness check in the warden is the only thing standing between a red
        # promotion and every consumer.
        self.assertIn("def is_green", warden)
        self.assertIn("PASSING_CONCLUSIONS", warden)
        # A stalled release has to surface rather than sit quietly, which is the
        # failure this whole workflow exists to prevent.
        self.assertIn("--alert-is-failure", workflow)
        self.assertIn("schedule:", workflow)

    def test_release_tag_is_not_hardcoded_in_the_build_workflow(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        release = (repo_root / ".github/workflows/unica-plugin-release.yml").read_text(
            encoding="utf-8"
        )
        # A hardcoded release version is a location no contract check covers, and
        # packaging fails on every later pull request once it drifts. A tag-shaped
        # literal is wrong anywhere in the file, however quoted, and this also
        # catches suffixed forms such as v1.2.3-rc1 by matching their prefix.
        tag_literals = sorted(set(re.findall(r"v\d+\.\d+\.\d+", release)))
        # An unprefixed literal is only wrong inside the step that derives the
        # tag, including in an intermediate variable it reads. The file elsewhere
        # pins other tools by bare version, so this cannot be a whole-file rule.
        step_name = "Resolve the release tag for non-tag builds"
        start = release.find(step_name)
        self.assertNotEqual(start, -1, "the workflow no longer derives the release tag")
        following = re.search(r"(?m)^      - (name|uses|run):", release[start:])
        step = release[start : start + following.start()] if following else release[start:]
        unprefixed = sorted(set(re.findall(r"\d+\.\d+\.\d+", step)))

        self.assertEqual(tag_literals, [])
        self.assertEqual(unprefixed, [])

    def test_bump_version_writes_every_contract_location(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        module_path = repo_root / "scripts" / "dev" / "bump-version.py"
        spec = importlib.util.spec_from_file_location("bump_version", module_path)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)

        contract = importlib.util.spec_from_file_location(
            "check_version_contract", repo_root / "scripts" / "ci" / "check-version-contract.py"
        )
        assert contract is not None and contract.loader is not None
        contract_module = importlib.util.module_from_spec(contract)
        contract.loader.exec_module(contract_module)

        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp) / "repo"
            for relative in (
                "Cargo.toml",
                "plugins/unica/.codex-plugin/plugin.json",
                "plugins/unica/third-party/tools.lock.json",
            ):
                target = work / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(
                    (repo_root / relative).read_text(encoding="utf-8"), encoding="utf-8"
                )
            # Synthesised rather than copied so the Claude manifest is covered on
            # branches that do not carry it yet.
            claude = work / "plugins/unica/.claude-plugin/plugin.json"
            claude.parent.mkdir(parents=True, exist_ok=True)
            claude.write_text(
                json.dumps({"name": "unica", "version": "0.0.0"}) + "\n", encoding="utf-8"
            )

            changed = module.bump(work, "9.8.7")
            values = contract_module.read_version_contract(work)
            claude_version = json.loads(claude.read_text(encoding="utf-8"))["version"]

        self.assertEqual(set(values.values()), {"9.8.7"}, values)
        self.assertEqual(claude_version, "9.8.7")
        self.assertIn("plugins/unica/.claude-plugin/plugin.json", changed)

    def test_bump_version_writes_nothing_when_a_later_file_is_malformed(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        module_path = repo_root / "scripts" / "dev" / "bump-version.py"
        spec = importlib.util.spec_from_file_location("bump_version", module_path)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)

        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp) / "repo"
            cargo = work / "Cargo.toml"
            cargo.parent.mkdir(parents=True, exist_ok=True)
            cargo.write_text(
                (repo_root / "Cargo.toml").read_text(encoding="utf-8"), encoding="utf-8"
            )
            lock = work / "plugins/unica/third-party/tools.lock.json"
            lock.parent.mkdir(parents=True, exist_ok=True)
            # Two unica entries: valid JSON, but no single version to set.
            lock.write_text(
                json.dumps({"tools": [{"name": "unica"}, {"name": "unica"}]}), encoding="utf-8"
            )
            before = cargo.read_text(encoding="utf-8")

            with self.assertRaises(SystemExit):
                module.bump(work, "9.8.7")

            # Straddling two versions is the exact state the contract forbids, so
            # a failure part-way through has to leave everything untouched.
            self.assertEqual(cargo.read_text(encoding="utf-8"), before)

    def test_promotion_pr_points_the_tag_at_the_staging_merge(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        publish = (repo_root / ".github/workflows/publish-unica-marketplace.yml").read_text(
            encoding="utf-8"
        )

        # Naming the promotion commit would require tagging a commit that does
        # not exist until the promotion step has already run, which leaves the
        # consumer install checks red on their first run every release.
        self.assertIn("staging merge commit ${STAGING_MERGE_SHA}", publish)
        self.assertNotIn("tag at commit ${promotion_sha}", publish)

    def test_readme_documents_public_marketplace_lifecycle(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        readme = (repo_root / "README.md").read_text(encoding="utf-8")

        required = (
            "codex plugin marketplace add IngvarConsulting/unica-marketplace --ref main",
            "codex plugin add unica@unica",
            "codex plugin marketplace upgrade unica",
            "codex plugin remove unica@unica",
            "codex plugin marketplace remove unica",
            "Git",
            "new Codex task",
            "SHA-256",
            "$CODEX_HOME/unica/runtimes",
        )
        for value in required:
            with self.subTest(value=value):
                self.assertIn(value, readme)

    def test_readme_documents_the_claude_marketplace_lifecycle(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        readme = (repo_root / "README.md").read_text(encoding="utf-8")

        required = (
            "claude plugin marketplace add IngvarConsulting/unica-marketplace",
            "claude plugin install unica@unica",
            "claude plugin marketplace update unica",
            "claude plugin update unica@unica",
            "claude plugin uninstall unica@unica",
            "claude plugin marketplace remove unica",
            "claude --plugin-dir ./plugins/unica",
            # The floor is load-bearing: older clients cannot parse git-subdir.
            "2.1.69",
        )
        for value in required:
            with self.subTest(value=value):
                self.assertIn(value, readme)

    def test_claude_host_contract_is_recorded_for_agents(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        agents = (repo_root / "AGENTS.md").read_text(encoding="utf-8")
        claude_md = (repo_root / "CLAUDE.md").read_text(encoding="utf-8")
        decisions = (repo_root / "spec/decisions/README.md").read_text(encoding="utf-8")

        self.assertIn("plugins/unica/.claude-plugin/plugin.json", agents)
        self.assertIn("AGENTS.md", claude_md)
        self.assertIn("0012-one-plugin-directory-for-two-hosts.md", decisions)
        self.assertTrue(
            (repo_root / "spec/decisions/0012-one-plugin-directory-for-two-hosts.md").is_file()
        )

    def test_publish_workflow_promotes_both_host_catalogs(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        publish = (repo_root / ".github/workflows/publish-unica-marketplace.yml").read_text(
            encoding="utf-8"
        )
        release = (repo_root / ".github/workflows/unica-plugin-release.yml").read_text(
            encoding="utf-8"
        )

        # Staging must carry both manifests, and promotion must move both
        # catalogs together, or one host would be left pointing at a stale tag.
        self.assertIn("payload/plugins/unica/.claude-plugin/plugin.json", publish)
        self.assertIn("payload/.claude-plugin/marketplace.json", publish)
        self.assertIn(
            "cp payload/.claude-plugin/marketplace.json "
            "marketplace/.claude-plugin/marketplace.json",
            publish,
        )
        # Copying is not enough: an unstaged catalog would leave the promotion
        # PR without the Claude entry while the copy assertion still passed.
        self.assertIn(
            "git -C marketplace add .agents/plugins/marketplace.json "
            ".claude-plugin/marketplace.json",
            publish,
        )
        # The gate is pinned to the compatibility floor, not to the latest CLI.
        self.assertIn("@anthropic-ai/claude-code@${CLAUDE_CLI_VERSION}", release)
        self.assertIn("CLAUDE_CLI_VERSION: 2.1.69", release)
        self.assertIn("claude plugin validate", release)

    def test_readme_documents_the_frozen_v078_bridge(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        readme = (repo_root / "README.md").read_text(encoding="utf-8")

        self.assertIn("| Ваша версия | Что делать |", readme)
        self.assertIn(
            "releases/download/v0.7.8/install-unica.sh",
            readme,
        )
        self.assertIn(
            "releases/download/v0.7.8/install-unica.ps1",
            readme,
        )
        self.assertIn("`0.7.5` и новее", readme)
        self.assertIn("v0.7.8", readme)
        self.assertIn("v0.8.0", readme)

    def test_active_consumer_docs_do_not_describe_fat_local_delivery(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        paths = [
            repo_root / "README.md",
            repo_root / "plugins/unica/README.md",
            repo_root / "spec/acceptance/unica-mcp-validation.md",
            repo_root / "spec/architecture/arc42/06-runtime-view.md",
            repo_root / "spec/architecture/arc42/07-deployment-view.md",
        ]
        forbidden = ("unica-local", "unica-codex-marketplace-")
        matches = [
            f"{path.relative_to(repo_root)}:{needle}"
            for path in paths
            for needle in forbidden
            if needle in path.read_text(encoding="utf-8")
        ]
        self.assertEqual(matches, [])

    def test_removed_script_backed_skills_do_not_leave_architecture_records(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        decisions = repo_root / "spec" / "decisions"
        index = (decisions / "README.md").read_text(encoding="utf-8")

        self.assertFalse((decisions / "0007-script-backed-utility-skill-exceptions.md").exists())
        self.assertFalse((decisions / "0009-remove-script-backed-utility-skills.md").exists())
        self.assertNotIn("Script-backed utility", index)

    def test_application_layer_does_not_spawn_git_directly(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        application_root = (
            repo_root / "crates" / "unica-coder" / "src" / "application"
        )
        offenders = []
        for path in application_root.rglob("*.rs"):
            production = path.read_text(encoding="utf-8").split(
                "#[cfg(test)]\nmod tests", maxsplit=1
            )[0]
            if 'std::process::Command::new("git")' in production:
                offenders.append(str(path.relative_to(repo_root)))

        self.assertEqual(offenders, [])

    def write_executable(self, tools_dir: Path, name: str, body: str) -> None:
        commands = {
            "bsl-analyzer": [("analyze", "--help"), ("mcp", "serve", "--help")],
            "rlm-bsl-index": [
                ("index", "build", "--help"),
                ("index", "update", "--help"),
                ("index", "info", "--help"),
            ],
            "rlm-tools-bsl": [("--help",)],
            "v8-runner": [("--version",), ("build", "--help")],
        }[name]
        routed_outputs = {
            tuple(route.split()): output
            for route, output in re.findall(
                r"'([^']+)'\) printf '%s\\n' '([^']*)'",
                body,
            )
        }
        fallback_outputs = re.findall(r"printf '%s\\n' '([^']*)'", body)
        fallback = fallback_outputs[0] if fallback_outputs else ""
        routes = {" ".join(command): routed_outputs.get(command, fallback) for command in commands}
        path = tools_dir / f"{name}.py"
        path.write_text(
            "#!/usr/bin/env python3\n"
            "import json\n"
            "import sys\n"
            f"ROUTES = json.loads({json.dumps(json.dumps(routes))})\n"
            "key = ' '.join(sys.argv[1:])\n"
            "if key not in ROUTES:\n"
            "    raise SystemExit(1)\n"
            "print(ROUTES[key])\n",
            encoding="utf-8",
        )
        path.chmod(path.stat().st_mode | 0o755)

    def test_tool_help_contracts_pass_with_expected_cli_surface(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            self.write_executable(
                tools_dir,
                "bsl-analyzer",
                self.BSL_ANALYZER_HELP,
            )
            self.write_executable(
                tools_dir,
                "rlm-bsl-index",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n",
            )
            self.write_executable(
                tools_dir,
                "rlm-tools-bsl",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_executable(
                tools_dir,
                "v8-runner",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner 0.5.1 version build'\n",
            )

            errors = module.check_tool_contracts(tools_dir)

        self.assertEqual(errors, [])

    def test_tool_help_contracts_accept_relative_tools_dir(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory(dir=Path.cwd()) as tmp:
            tools_dir = Path(tmp)
            self.write_executable(
                tools_dir,
                "bsl-analyzer",
                self.BSL_ANALYZER_HELP,
            )
            self.write_executable(
                tools_dir,
                "rlm-bsl-index",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n",
            )
            self.write_executable(
                tools_dir,
                "rlm-tools-bsl",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_executable(
                tools_dir,
                "v8-runner",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner 0.5.1 version build'\n",
            )

            errors = module.check_tool_contracts(tools_dir.relative_to(Path.cwd()))

        self.assertEqual(errors, [])

    def test_tool_help_contracts_report_missing_expected_flag(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            self.write_executable(tools_dir, "bsl-analyzer", "#!/usr/bin/env sh\nprintf '%s\\n' 'analyze'\n")
            self.write_executable(tools_dir, "rlm-bsl-index", "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n")
            self.write_executable(
                tools_dir,
                "rlm-tools-bsl",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_executable(tools_dir, "v8-runner", "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner version build'\n")

            errors = module.check_tool_contracts(tools_dir)

        self.assertTrue(any("--source-dir" in error for error in errors), errors)

    def test_analyze_help_cannot_borrow_tokens_from_mcp_serve_help(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            self.write_executable(
                tools_dir,
                "bsl-analyzer",
                "#!/usr/bin/env sh\n"
                "case \"$*\" in\n"
                "  'analyze --help') printf '%s\\n' '--format jsonl' ;;\n"
                "  'mcp serve --help') printf '%s\\n' '--profile --source-dir --mode stdio' ;;\n"
                "  *) exit 1 ;;\n"
                "esac\n",
            )
            self.write_executable(
                tools_dir,
                "rlm-bsl-index",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n",
            )
            self.write_executable(
                tools_dir,
                "rlm-tools-bsl",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_executable(
                tools_dir,
                "v8-runner",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner version build'\n",
            )

            errors = module.check_tool_contracts(tools_dir)

        self.assertTrue(
            any("bsl-analyzer analyze" in error and "--source-dir" in error for error in errors),
            errors,
        )

    def test_runtime_docs_define_workspace_service_deadlines_exactly(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        runtime = (repo_root / "spec" / "architecture" / "arc42" / "06-runtime-view.md").read_text(
            encoding="utf-8"
        )
        acceptance = (repo_root / "spec" / "acceptance" / "unica-mcp-validation.md").read_text(
            encoding="utf-8"
        )
        adr = (repo_root / "spec" / "decisions" / "0006-workspace-scoped-internal-services.md").read_text(
            encoding="utf-8"
        )

        for text in (runtime, acceptance, adr):
            normalized = " ".join(text.split())
            self.assertIn("120-second overall deadline", normalized)
            self.assertIn("500 ms connect cap", normalized)
            self.assertIn("remaining overall budget", normalized)
            self.assertIn("best-effort `Cancel`", normalized)
            self.assertIn("separate 500 ms aggregate budget", normalized)
            self.assertIn("connect, write, flush, and read", normalized)
            self.assertIn("does not read a response", normalized)
            self.assertIn("cancellation takes precedence", normalized)
            self.assertIn("100 ms", normalized)

    def test_tool_help_contracts_report_missing_rlm_server_transport_surface(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            self.write_executable(
                tools_dir,
                "bsl-analyzer",
                self.BSL_ANALYZER_HELP,
            )
            self.write_executable(tools_dir, "rlm-bsl-index", "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n")
            self.write_executable(tools_dir, "rlm-tools-bsl", "#!/usr/bin/env sh\nprintf '%s\\n' 'service'\n")
            self.write_executable(tools_dir, "v8-runner", "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner version build'\n")

            errors = module.check_tool_contracts(tools_dir)

        self.assertTrue(any("rlm-tools-bsl server" in error and "--transport" in error for error in errors), errors)

    def test_rlm_schema_contract_checks_tables_meta_and_columns_used_by_unica_sql(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "bsl_index.db"
            with closing(sqlite3.connect(db_path)) as conn, conn:
                conn.execute("CREATE TABLE index_meta (key TEXT PRIMARY KEY, value TEXT)")
                conn.execute("INSERT INTO index_meta (key, value) VALUES ('builder_version', '14')")
                conn.execute(
                    "CREATE TABLE modules (id INTEGER, rel_path TEXT, object_name TEXT, "
                    "category TEXT, module_type TEXT)"
                )
                conn.execute(
                    "CREATE TABLE methods (id INTEGER, module_id INTEGER, name TEXT, type TEXT, "
                    "is_export INTEGER, line INTEGER, end_line INTEGER, params TEXT, loc INTEGER)"
                )
                conn.execute("CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name)")
                conn.execute(
                    "CREATE TABLE regions (id INTEGER, module_id INTEGER, name TEXT, "
                    "line INTEGER, end_line INTEGER)"
                )
                conn.execute("CREATE TABLE module_headers (module_id INTEGER, header_comment TEXT)")
                conn.execute(
                    "CREATE TABLE object_attributes (id INTEGER, object_name TEXT, category TEXT, "
                    "attr_name TEXT, attr_synonym TEXT, attr_type TEXT, attr_kind TEXT, "
                    "ts_name TEXT, source_file TEXT)"
                )
                conn.execute(
                    "CREATE TABLE role_rights (id INTEGER, role_name TEXT, object_name TEXT, "
                    "right_name TEXT, file TEXT)"
                )
                conn.execute(
                    "CREATE TABLE event_subscriptions (id INTEGER, name TEXT, synonym TEXT, "
                    "event TEXT, handler_module TEXT, handler_procedure TEXT, source_types TEXT, "
                    "source_count INTEGER, file TEXT)"
                )
                conn.execute(
                    "CREATE TABLE functional_options (id INTEGER, name TEXT, synonym TEXT, "
                    "location TEXT, content TEXT, file TEXT)"
                )
                conn.execute(
                    "CREATE TABLE predefined_items (id INTEGER, object_name TEXT, category TEXT, "
                    "item_name TEXT, item_synonym TEXT, item_code TEXT, types_json TEXT, "
                    "is_folder INTEGER, source_file TEXT)"
                )

            self.assertEqual(module.check_rlm_schema(db_path), [])

    def test_rlm_mtime_recovery_contract_checks_scripted_orchestration(self) -> None:
        module = load_contract_module()
        outputs = iter(
            [
                (0, "Index built\n"),
                (0, "Status: fresh\n"),
                (0, "Status: stale (content)\n"),
                (0, "Changed: 0\nFast path: True\n"),
                (0, "Status: stale (content)\n"),
                (0, "Index built\n"),
                (0, "Status: fresh\n"),
            ]
        )
        actions = []

        def run_rlm(command, cwd, env):
            action = command[2]
            self.assertEqual(
                command,
                ["rlm-bsl-index", "index", action, str(cwd)],
            )
            actions.append(command[2])
            self.assertEqual(cwd, Path(command[3]))
            self.assertEqual(env["RLM_INDEX_DIR"], str(cwd.parent / "index"))
            self.assertEqual(env["RLM_INDEX_SAMPLE_SIZE"], "1000")
            self.assertEqual(env["RLM_INDEX_SAMPLE_THRESHOLD"], "0")
            self.assertEqual(env["RLM_INDEX_SKIP_SAMPLE_HOURS"], "0")
            return next(outputs)

        errors = module.check_rlm_mtime_recovery_contract(
            Path("rlm-bsl-index"),
            run_rlm=run_rlm,
        )

        self.assertEqual(errors, [])
        self.assertEqual(
            actions,
            ["build", "info", "info", "update", "info", "build", "info"],
        )

    def test_run_rlm_command_times_out_instead_of_hanging(self) -> None:
        module = load_contract_module()
        timeout = module.subprocess.TimeoutExpired(["rlm-bsl-index"], 120.0)

        with patch.object(module.subprocess, "run", side_effect=timeout) as run:
            status, output = module.run_rlm_command(
                ["rlm-bsl-index"],
                Path.cwd(),
                {},
            )

        self.assertEqual(status, 1)
        self.assertIn("timed out after 120.0s", output)
        self.assertEqual(run.call_args.kwargs["timeout"], 120.0)

    def test_run_rlm_command_reuses_script_wrapping(self) -> None:
        module = load_contract_module()
        completed = module.subprocess.CompletedProcess(
            ["fixture.py"],
            0,
            stdout="wrapped stdout\n",
            stderr="wrapped stderr\n",
        )

        with patch.object(module.subprocess, "run", return_value=completed) as run:
            status, output = module.run_rlm_command(
                ["fixture.py", "index", "info"],
                Path.cwd(),
                {"RLM_CONTRACT_TEST": "1"},
            )

        self.assertEqual(status, 0)
        self.assertEqual(output, "wrapped stdout\nwrapped stderr\n")
        wrapped_command = run.call_args.args[0]
        self.assertEqual(wrapped_command[0], module.sys.executable)
        self.assertEqual(wrapped_command[1:], ["fixture.py", "index", "info"])
        self.assertEqual(
            run.call_args.kwargs["env"]["RLM_CONTRACT_TEST"],
            "1",
        )
        self.assertEqual(run.call_args.kwargs["timeout"], 120.0)

    def test_rlm_mtime_recovery_fixture_disables_git_signing(self) -> None:
        module = load_contract_module()
        outputs = iter(
            [
                (0, "Index built\n"),
                (0, "Status: fresh\n"),
                (0, "Status: stale (content)\n"),
                (0, "Changed: 0\nFast path: True\n"),
                (0, "Status: stale (content)\n"),
                (0, "Index built\n"),
                (0, "Status: fresh\n"),
            ]
        )
        git_commands = []

        def run_git(command, cwd):
            git_commands.append(command)
            if command == ["git", "rev-parse", "HEAD"]:
                return 0, "fixture-head\n"
            return 0, ""

        with patch.object(module, "run_command", side_effect=run_git):
            errors = module.check_rlm_mtime_recovery_contract(
                Path("rlm-bsl-index"),
                run_rlm=lambda command, cwd, env: next(outputs),
            )

        self.assertEqual(errors, [])
        signing_disabled = [
            "git",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgSign=false",
        ]
        self.assertEqual(
            git_commands,
            [
                [*signing_disabled, "init", "-q"],
                [
                    *signing_disabled,
                    "config",
                    "user.email",
                    "unica-ci@example.invalid",
                ],
                [*signing_disabled, "config", "user.name", "Unica CI"],
                [*signing_disabled, "add", "."],
                [*signing_disabled, "commit", "-q", "-m", "fixture"],
                ["git", "status", "--porcelain", "--untracked-files=no"],
                ["git", "rev-parse", "HEAD"],
                ["git", "rev-parse", "HEAD"],
            ],
        )

    def test_rlm_mtime_recovery_contract_rejects_changed_git_head(self) -> None:
        module = load_contract_module()
        outputs = iter(
            [
                (0, "Index built\n"),
                (0, "Status: fresh\n"),
                (0, "Status: stale (content)\n"),
                (0, "Changed: 0\nFast path: True\n"),
                (0, "Status: stale (content)\n"),
                (0, "Index built\n"),
                (0, "Status: fresh\n"),
            ]
        )
        heads = iter(["initial-head\n", "changed-head\n"])

        def run_git(command, cwd):
            if command == ["git", "rev-parse", "HEAD"]:
                return 0, next(heads)
            return 0, ""

        with patch.object(module, "run_command", side_effect=run_git):
            errors = module.check_rlm_mtime_recovery_contract(
                Path("rlm-bsl-index"),
                run_rlm=lambda command, cwd, env: next(outputs),
            )

        self.assertTrue(
            any("Git HEAD changed during update" in error for error in errors),
            errors,
        )

    def test_rlm_schema_contract_reports_missing_column(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "bsl_index.db"
            with closing(sqlite3.connect(db_path)) as conn, conn:
                conn.execute("CREATE TABLE index_meta (key TEXT PRIMARY KEY, value TEXT)")
                conn.execute("INSERT INTO index_meta (key, value) VALUES ('builder_version', '14')")
                conn.execute("CREATE TABLE modules (id INTEGER, rel_path TEXT)")
                conn.execute("CREATE TABLE methods (id INTEGER, module_id INTEGER, name TEXT)")
                conn.execute("CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name)")
                conn.execute(
                    "CREATE TABLE regions (id INTEGER, module_id INTEGER, name TEXT, "
                    "line INTEGER, end_line INTEGER)"
                )
                conn.execute("CREATE TABLE module_headers (module_id INTEGER, header_comment TEXT)")

            errors = module.check_rlm_schema(db_path)

        self.assertTrue(any("modules.object_name" in error for error in errors), errors)

    def test_rlm_schema_contract_requires_metadata_tables_used_by_meta_profile(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "bsl_index.db"
            with closing(sqlite3.connect(db_path)) as conn, conn:
                conn.execute("CREATE TABLE index_meta (key TEXT PRIMARY KEY, value TEXT)")
                conn.execute("INSERT INTO index_meta (key, value) VALUES ('builder_version', '14')")
                conn.execute(
                    "CREATE TABLE modules (id INTEGER, rel_path TEXT, object_name TEXT, "
                    "category TEXT, module_type TEXT)"
                )
                conn.execute(
                    "CREATE TABLE methods (id INTEGER, module_id INTEGER, name TEXT, type TEXT, "
                    "is_export INTEGER, line INTEGER, end_line INTEGER, params TEXT, loc INTEGER)"
                )
                conn.execute("CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name)")
                conn.execute(
                    "CREATE TABLE regions (id INTEGER, module_id INTEGER, name TEXT, "
                    "line INTEGER, end_line INTEGER)"
                )
                conn.execute("CREATE TABLE module_headers (module_id INTEGER, header_comment TEXT)")

            errors = module.check_rlm_schema(db_path)

        self.assertTrue(any("role_rights" in error for error in errors), errors)
        self.assertTrue(any("object_attributes" in error for error in errors), errors)
        self.assertTrue(any("functional_options" in error for error in errors), errors)

    def test_rlm_schema_contract_reports_old_builder_version(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            db_path = Path(tmp) / "bsl_index.db"
            with closing(sqlite3.connect(db_path)) as conn, conn:
                conn.execute("CREATE TABLE index_meta (key TEXT PRIMARY KEY, value TEXT)")
                conn.execute("INSERT INTO index_meta (key, value) VALUES ('builder_version', '12')")
                conn.execute(
                    "CREATE TABLE modules (id INTEGER, rel_path TEXT, object_name TEXT, "
                    "category TEXT, module_type TEXT)"
                )
                conn.execute(
                    "CREATE TABLE methods (id INTEGER, module_id INTEGER, name TEXT, type TEXT, "
                    "is_export INTEGER, line INTEGER, end_line INTEGER, params TEXT, loc INTEGER)"
                )
                conn.execute("CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name)")
                conn.execute(
                    "CREATE TABLE regions (id INTEGER, module_id INTEGER, name TEXT, "
                    "line INTEGER, end_line INTEGER)"
                )
                conn.execute("CREATE TABLE module_headers (module_id INTEGER, header_comment TEXT)")

            errors = module.check_rlm_schema(db_path)

        self.assertTrue(any("builder_version" in error and "14" in error for error in errors), errors)


if __name__ == "__main__":
    unittest.main()
