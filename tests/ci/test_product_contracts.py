from __future__ import annotations

import importlib.util
import json
import re
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[2]


def load_contract_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "check-tool-contracts.py"
    spec = importlib.util.spec_from_file_location("check_tool_contracts", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ProductContractTests(unittest.TestCase):
    def test_runtime_worker_handoff_documentation_names_both_frames(self) -> None:
        runtime = (REPO_ROOT / "spec/architecture/runtime.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("два JSON-документа", runtime)
        self.assertIn("подтверждение запуска", runtime)

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

    def test_v8_runner_platform_stub_compilation_timeout_is_bounded(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "platform-stub.rs"
            output = root / "platform-stub.exe"
            source.write_text("fn main() {}\n", encoding="utf-8")
            with patch.object(
                module.subprocess,
                "run",
                side_effect=subprocess.TimeoutExpired(["rustc"], 60),
            ) as compile_run:
                errors = module.compile_rust_platform_stub(
                    source,
                    output,
                    root,
                    "v8-runner fixture",
                )

        self.assertEqual(
            errors,
            [
                "v8-runner fixture: platform stub compilation timed out "
                "after 60 seconds"
            ],
        )
        self.assertEqual(compile_run.call_args.kwargs["timeout"], 60)

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

    def test_v8_runner_windows_external_publication_result_accepts_clean_epf(
        self,
    ) -> None:
        module = load_contract_module()
        validator = getattr(
            module,
            "validate_v8_runner_windows_external_publication_result",
            None,
        )
        self.assertIsNotNone(validator)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / "Deploy"
            epf = output / "Alpha.epf"
            output.mkdir()
            epf.write_bytes(b"issue-310-current")
            envelope = {
                "ok": True,
                "command": "make",
                "data": {
                    "ok": True,
                    "mode": "external_data_processor_epf",
                    "source_set": "external-processors",
                    "output_path": "Deploy",
                    "artifacts": {
                        "root_dir": "Deploy",
                        "items": [
                            {
                                "kind": "package",
                                "path": str(Path("Deploy") / "Alpha.epf"),
                                "role": "package_file",
                            }
                        ],
                    },
                    "execution": {
                        "status": "succeeded",
                        "payload": {
                            "artifact_type": "external_data_processor_epf",
                            "output_path": "Deploy",
                            "file_names": ["Alpha.epf"],
                            "published": True,
                        },
                    },
                },
            }

            self.assertEqual(
                validator(envelope, output, epf, b"issue-310-current", root),
                [],
            )

    def test_v8_runner_windows_external_publication_result_rejects_failed_or_dirty_publish(
        self,
    ) -> None:
        module = load_contract_module()
        validator = module.validate_v8_runner_windows_external_publication_result

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / "Deploy"
            epf = output / "Alpha.epf"
            output.mkdir()
            epf.write_bytes(b"issue-310-stale")
            (root / ".artifacts-stage-leftover").mkdir()
            envelope = {
                "ok": False,
                "data": {
                    "ok": False,
                    "mode": "external_data_processor_epf",
                    "source_set": "external-processors",
                    "output_path": str(output),
                    "execution": {"status": "failed"},
                },
            }

            errors = validator(
                envelope,
                output,
                epf,
                b"issue-310-current",
                root,
            )

        self.assertTrue(any("envelope" in error for error in errors), errors)
        self.assertTrue(any("unexpected bytes" in error for error in errors), errors)
        self.assertTrue(any("temporary state" in error for error in errors), errors)

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

    def test_targeted_tool_contracts_run_windows_external_publication_smoke(
        self,
    ) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            runner = tools_dir / "v8-runner.exe"
            runner.write_bytes(b"runner")
            with (
                patch.object(module, "TOOL_HELP_CHECKS", []),
                patch.object(
                    module,
                    "check_v8_runner_partial_load_contract",
                    return_value=[],
                ),
                patch.object(
                    module,
                    "check_v8_runner_bounded_external_epf_contract",
                    return_value=[],
                ),
                patch.object(
                    module,
                    "check_v8_runner_windows_external_publication_contract",
                    return_value=["windows publication failure"],
                ) as publication_check,
            ):
                errors = module.check_tool_contracts(tools_dir, "win-x64")

        self.assertEqual(errors, ["windows publication failure"])
        publication_check.assert_called_once_with(runner.resolve(), "win-x64")

    BSL_ANALYZER_HELP = (
        "#!/usr/bin/env sh\n"
        "case \"$*\" in\n"
        "  'analyze --help') printf '%s\\n' '--source-dir --format jsonl' ;;\n"
        "  'mcp serve --help') printf '%s\\n' '--profile --source-dir --mode stdio' ;;\n"
        "  *) exit 1 ;;\n"
        "esac\n"
    )

    def test_task_router_paths_resolve(self) -> None:
        """Каждый путь таблицы маршрутизации указывает на существующее место.

        Таблица — единственный маршрут от задачи к коду, и её пути записаны в
        обратных кавычках, а не markdown-ссылками, поэтому резолвер ссылок их
        не видел. Шесть строк успели усохнуть до хвоста вроде
        `domain/cache.rs`: от корня такой путь не разрешается, `rg` по нему
        ничего не находит, и строка перестаёт быть маршрутом.
        """
        repo_root = Path(__file__).resolve().parents[2]
        agents = (repo_root / "AGENTS.md").read_text(encoding="utf-8")

        section = agents.split("## Куда смотреть, где менять", 1)[1].split("\n## ", 1)[0]
        rows = [
            line
            for line in section.splitlines()
            if line.startswith("|") and not set(line) <= set("| -")
        ]
        self.assertGreater(len(rows), 5, "таблица маршрутизации не разобрана")

        # Только два префикса читаются от `spec/`, и оба названы в шапке
        # таблицы; всё остальное — от корня репозитория.
        spec_relative = ("architecture/", "acceptance/")
        extensions = (".md", ".rs", ".py", ".yml", ".yaml", ".json", ".toml")
        offenders = []
        checked = 0
        for row in rows[1:]:
            for token in re.findall(r"`([^`]+)`", row):
                if "/" not in token and not token.endswith(extensions):
                    continue
                # `<группа>` и `<имя>` подставляются вызывающим; проверяется
                # каталог, в котором такой файл обязан лежать.
                probe = token
                if "<" in probe:
                    probe = probe.rsplit("/", 1)[0] if "/" in probe else probe
                    if "<" in probe:
                        continue
                base = repo_root / "spec" if probe.startswith(spec_relative) else repo_root
                checked += 1
                if not (base / probe).exists():
                    offenders.append(token)

        self.assertGreater(checked, 15, "пути таблицы не разобраны")
        self.assertEqual(offenders, [], "путь из таблицы маршрутизации не разрешается")

    def test_downloader_and_local_corpus_contract_are_retired(self) -> None:
        """Справка платформы приходит из установки, а не из скачанного корпуса.

        Загрузчик закреплял ровно ту болезнь, ради которой заведена #254:
        полная загрузка в каждом рабочем дереве ради точечного вопроса.
        """
        downloader = REPO_ROOT / "scripts" / "dev" / "download-1ci-guides.py"
        downloader_test = REPO_ROOT / "tests" / "dev" / "test_download_1ci_guides.py"
        agents = (REPO_ROOT / "AGENTS.md").read_text(encoding="utf-8")

        self.assertFalse(downloader.exists(), "загрузчик удалён вместе с контрактом корпуса")
        self.assertFalse(downloader_test.exists(), "тест загрузчика удалён вместе с ним")
        self.assertNotIn("download-1ci-guides.py", agents)
        self.assertNotIn("docs-local/1ci/8.3.27/en/", agents)
        self.assertNotIn("kb.1ci.com/bin/download", agents)
        # Активный слой spec/ — не только AGENTS.md: указание на локальный
        # корпус в нём отправляет читателя к пути, который больше ничем не
        # создаётся. Исторические docs/design и docs/plans сюда не входят.
        for spec_path in sorted((REPO_ROOT / "spec").rglob("*")):
            if not spec_path.is_file():
                continue
            with self.subTest(path=spec_path.relative_to(REPO_ROOT).as_posix()):
                self.assertNotIn(
                    "docs-local/1ci",
                    spec_path.read_text(encoding="utf-8"),
                    "активный слой spec не должен ссылаться на снятый корпус",
                )

    def test_local_corpus_directory_stays_ignored(self) -> None:
        """Каталог остаётся игнорируемым: снят контракт корпуса, а не каталог."""
        ignore = (REPO_ROOT / ".gitignore").read_text(encoding="utf-8")
        self.assertIn("docs-local/", ignore.splitlines())

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
        )
        for value in required:
            with self.subTest(value=value):
                self.assertIn(value, readme)

    def test_claude_version_floor_stays_recorded_outside_the_root_readme(self) -> None:
        # The floor is load-bearing: clients before 2.1.69 cannot parse the
        # catalog's git-subdir source. The root README deliberately omits it,
        # so the plugin README and the decision record must keep it.
        repo_root = Path(__file__).resolve().parents[2]
        plugin_readme = (repo_root / "plugins/unica/README.md").read_text(encoding="utf-8")
        decision = (
            repo_root / "spec/decisions/0012-one-plugin-directory-for-two-hosts.md"
        ).read_text(encoding="utf-8")

        self.assertIn("2.1.69", plugin_readme)
        self.assertIn("2.1.69", decision)

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
            repo_root / "spec/architecture/runtime.md",
            repo_root / "spec/architecture/deployment.md",
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
        """Three documents must quote the budgets the runtime actually enforces.

        The contract is the numbers, not the wording. Asserting exact English
        prose broke the moment the architecture layer was translated, while the
        thing worth protecting is language-independent.

        The budgets are read from the constants that enforce them rather than
        written down here a second time. A literal set inside the test made the
        check circular in two ways: changing `SERVICE_REQUEST_TIMEOUT` in the
        runtime left every document and this test green, and the two former
        "documents agree" assertions could not fail at all -- the loop above
        them already established `required <= found` for every document, so
        `found & required` was `required` on both sides of each comparison.
        Anchoring the set to the source makes the loop the real check: drift in
        the runtime now fails against all three documents at once.
        """
        repo_root = Path(__file__).resolve().parents[2]
        sources = {
            "runtime": repo_root / "spec/architecture/runtime.md",
            "acceptance": repo_root / "spec/acceptance/unica-mcp-validation.md",
            "adr-0006": repo_root
            / "spec/decisions/0006-workspace-scoped-internal-services.md",
        }

        # Quantity plus unit, in either language: "120 seconds", "500 мс", "8 MiB".
        quantity = re.compile(
            r"(?<![\w.])(\d+)[\s-]*"
            r"(seconds?|секунд\w*|ms\b|мс\b|MiB|КиБ|KiB|МиБ)",
            re.IGNORECASE,
        )

        budgets = {}
        for label, path in sources.items():
            text = " ".join(path.read_text(encoding="utf-8").split())
            found = set()
            for value, unit in quantity.findall(text):
                unit = unit.lower()
                if unit.startswith(("second", "секунд")):
                    canonical = "s"
                elif unit in {"ms", "мс"}:
                    canonical = "ms"
                else:
                    canonical = "bytes"
                found.add(f"{value}{canonical}")
            budgets[label] = found

        required = self.enforced_workspace_service_budgets()
        for label, found in budgets.items():
            missing = sorted(required - found)
            self.assertEqual(
                missing, [], f"{label} no longer states the budgets {missing}"
            )

    def enforced_workspace_service_budgets(self) -> set:
        """The deadlines the workspace service enforces, read from its source.

        Two are named constants; the read-poll slice is a literal repeated at
        every polling call site, so it is taken from those call sites and must
        be a single value -- several different slices would mean the documented
        one describes only part of the behaviour.

        The unit is captured rather than assumed. Matching only `from_millis`
        would let a call site move to `from_secs` and simply drop out of the
        set, leaving the remaining site to agree with the documents on its own
        while the runtime had stopped behaving as documented. Test code is cut
        away first: the fixtures set their own read timeouts, and those are not
        budgets anyone documents.
        """
        repo_root = Path(__file__).resolve().parents[2]
        source = (
            repo_root / "crates/unica-coder/src/infrastructure/workspace_services.rs"
        ).read_text(encoding="utf-8")
        source = re.split(r"^#\[cfg\(test\)\]", source, maxsplit=1, flags=re.MULTILINE)[0]

        constants = {
            match.group("name"): (match.group("unit"), match.group("value"))
            for match in re.finditer(
                r"^const (?P<name>[A-Z_]+): Duration = "
                r"Duration::from_(?P<unit>secs|millis)\((?P<value>\d+)\);",
                source,
                re.MULTILINE,
            )
        }
        budgets = set()
        for name in ("SERVICE_REQUEST_TIMEOUT", "SERVICE_CONTROL_CONNECT_TIMEOUT"):
            self.assertIn(name, constants, f"{name} no longer names a duration")
            unit, value = constants[name]
            budgets.add(f"{value}{'s' if unit == 'secs' else 'ms'}")

        slices = {
            f"{value}{'s' if unit == 'secs' else 'ms'}"
            for unit, value in re.findall(
                r"set_read_timeout\(\s*Some\((?:remaining\.min\()?"
                r"Duration::from_(secs|millis)\((\d+)\)",
                source,
            )
        }
        self.assertEqual(
            len(slices), 1, f"read polling uses several slice lengths: {sorted(slices)}"
        )
        budgets.add(slices.pop())
        return budgets

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

    def test_tool_contract_checker_does_not_depend_on_rlm_sqlite_schema(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        checker = (repo_root / "scripts" / "ci" / "check-tool-contracts.py").read_text(
            encoding="utf-8"
        )
        for removed in ("sqlite3", "RLM_SCHEMA_COLUMNS", "check_rlm_schema", "--rlm-db"):
            self.assertNotIn(removed, checker)

        lock = tomllib.loads((repo_root / "Cargo.lock").read_text(encoding="utf-8"))
        dependency_names = {package["name"] for package in lock["package"]}
        self.assertEqual(
            sorted(name for name in dependency_names if "sqlite" in name.lower()),
            [],
        )

        rust_roots = [
            repo_root / "crates" / "unica-coder" / "src",
            repo_root / "crates" / "unica-bootstrap" / "src",
        ]
        production = "\n".join(
            path.read_text(encoding="utf-8")
            for rust_root in rust_roots
            for path in sorted(rust_root.rglob("*.rs"))
        )
        for removed in (
            "rusqlite",
            "libsqlite3",
            "sqlite3",
            "Connection::open",
            "methods_fts",
        ):
            self.assertNotIn(removed, production)

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

if __name__ == "__main__":
    unittest.main()
