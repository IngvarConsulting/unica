from __future__ import annotations

import copy
from contextlib import nullcontext
import gc
import importlib.util
import json
import os
import re
import signal
import subprocess
import tempfile
import time
import tomllib
import unittest
import warnings
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

    def test_v8_runner_failed_partial_receipt_requires_exit_four_and_closed_shape(
        self,
    ) -> None:
        module = load_contract_module()
        validator = getattr(
            module,
            "validate_v8_runner_failed_partial_receipt",
            None,
        )
        self.assertIsNotNone(validator)
        message = (
            "load failed for source-set 'main' with exit code 1; "
            "platform log: rejected; platform log path: /tmp/out.log; "
            "partial load list path: /tmp/partial.lst"
        )
        envelope = {
            "ok": False,
            "command": "build",
            "duration_ms": 12,
            "data": {
                "ok": False,
                "steps": [
                    {
                        "source_set": "main",
                        "mode": {"partial": {"file_count": 1}},
                        "ok": False,
                        "message": f"platform error: {message}",
                        "duration_ms": 0,
                    }
                ],
                "duration_ms": 12,
            },
            "warnings": [],
            "steps": [],
            "error": {
                "code": "platform_failure",
                "kind": "platform",
                "message": message,
            },
        }

        self.assertEqual(validator(envelope, 4, "main"), [])
        self.assertTrue(
            any("exit code 4" in error for error in validator(envelope, 1, "main"))
        )
        invalid_duration = copy.deepcopy(envelope)
        invalid_duration["data"]["steps"][0]["duration_ms"] = "0"
        self.assertTrue(
            any(
                "duration_ms" in error
                for error in validator(invalid_duration, 4, "main")
            )
        )
        unknown_field = copy.deepcopy(envelope)
        unknown_field["data"]["steps"][0]["mode"]["partial"]["unknown"] = True
        self.assertTrue(
            any("closed" in error for error in validator(unknown_field, 4, "main"))
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
            "bump-version.py",
            "check-version-contract.py",
            "publish-unica-marketplace.yml",
            # One human action starts the pipeline; the runbook must say which.
            "git tag -s vX.Y.Z",
            "stage → tag → verify → promote",
            # A release that fails part-way has to have a documented way out.
            "One-way doors",
            "never reuse a version number",
            "Rolling back a live release",
        ):
            with self.subTest(value=value):
                self.assertIn(value, text)

    def test_the_catalog_moves_only_behind_green_consumer_installs(self) -> None:
        """The linear pipeline replaces the warden's greenness check (ADR-0068).

        The marketplace default branch has no protection rules, so the job
        ordering here is the only thing standing between unverified bytes and
        every consumer: promote must require the install checks, the anchor tag
        must exist before them, and staging must never touch the catalog.
        """
        repo_root = Path(__file__).resolve().parents[2]
        publish = (repo_root / ".github/workflows/publish-unica-marketplace.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("needs: [stage, tag, verify-fresh-install, verify-upgrade]", publish)
        self.assertIn("needs: [stage, tag]", publish)
        # Staging pushes the payload only; the catalog files move in promote.
        self.assertNotIn("stage: Unica catalog", publish)
        self.assertIn("without changing the stable catalog", publish)
        # A published tag never moves; a rerun proves sameness instead.
        self.assertNotIn("git tag -f", publish)
        self.assertNotIn("--force", publish)
        # No scheduled babysitter exists anymore; the pipeline is one pass.
        self.assertFalse((repo_root / ".github/workflows/release-warden.yml").exists())
        self.assertFalse((repo_root / "scripts/ci/release-warden.py").exists())

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

    def test_the_anchor_tag_names_the_staging_commit(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        publish = (repo_root / ".github/workflows/publish-unica-marketplace.yml").read_text(
            encoding="utf-8"
        )

        # Naming the promotion commit would require tagging a commit that does
        # not exist until the catalog has already moved, which is exactly the
        # order the two-phase invariant forbids. The staging commit carries the
        # plugin bytes and exists before the install checks run.
        self.assertIn('tag -a "$RELEASE_TAG" "$STAGING_SHA"', publish)
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

    def test_active_delivery_docs_describe_core_first_and_digest_cache(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        root_readme = (repo_root / "README.md").read_text(encoding="utf-8")
        plugin_readme = (repo_root / "plugins/unica/README.md").read_text(
            encoding="utf-8"
        )
        internal = (repo_root / "docs/internal-package.md").read_text(encoding="utf-8")

        for marker in (
            "При старте MCP bootstrap скачивает только ядро",
            "<artifact>/<version>--<asset-sha256>/<target>",
            "unica-bootstrap prefetch --plugin-root",
        ):
            with self.subTest(document="README.md", marker=marker):
                self.assertIn(marker, root_readme)
        for marker in (
            "The bootstrap downloads only `unica-runtime-<target>.tar.gz` before MCP startup",
            "`<artifact>/<version>--<asset-sha256>/<target>`",
            "`work.status=working`",
            "unica-bootstrap prefetch --plugin-root",
        ):
            with self.subTest(document="plugins/unica/README.md", marker=marker):
                self.assertIn(marker, plugin_readme)
        for marker in (
            "`<cacheRoot>/<artifact>/<version>--<assetSha256>/<target>`",
            "`ensure_artifact`",
            "`prefetch`",
        ):
            with self.subTest(document="docs/internal-package.md", marker=marker):
                self.assertIn(marker, internal)

        forbidden = (
            "$CODEX_HOME/unica/runtimes/<version>/<target>",
            "${CLAUDE_PLUGIN_DATA}/runtimes/<version>/<target>",
            "The runtime archive contains the target's",
        )
        matches = [
            marker
            for text in (root_readme, plugin_readme, internal)
            for marker in forbidden
            if marker in text
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
            "rlm-bsl-mcp": [("--help",)],
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

    def write_rlm_contract_standins(
        self,
        root: Path,
        *,
        mode: str = "ok",
        index_mode: str = "ok",
    ) -> tuple[Path, Path, Path, Path, Path]:
        request_log = root / "requests.jsonl"
        index_log = root / "index.json"
        pid_log = root / "pids.txt"
        index_tool = root / "rlm-bsl-index.py"
        index_tool.write_text(
            "import json, os, subprocess, sys, time\n"
            "from pathlib import Path\n"
            f"INDEX_MODE = {index_mode!r}\n"
            "workspace = Path(sys.argv[3]).resolve()\n"
            "if INDEX_MODE == 'reject_outer_scan' and "
            "(workspace.parents[1] / 'poison-outer-sibling').exists():\n"
            "    raise SystemExit(19)\n"
            "if INDEX_MODE == 'hang':\n"
            "    child = subprocess.Popen([sys.executable, '-c', "
            "'import time; time.sleep(30)'])\n"
            f"    Path({str(pid_log)!r}).write_text("
            "f'{os.getpid()}\\n{child.pid}\\n', encoding='utf-8')\n"
            "    time.sleep(30)\n"
            f"Path({str(index_log)!r}).write_text("
            "json.dumps({'argv': sys.argv[1:], "
            "'index_dir': os.environ.get('RLM_INDEX_DIR')}), encoding='utf-8')\n"
            "Path(os.environ['RLM_INDEX_DIR']).mkdir(parents=True, exist_ok=True)\n",
            encoding="utf-8",
        )
        index_tool.chmod(index_tool.stat().st_mode | 0o755)

        mcp_tool = root / "rlm-bsl-mcp.py"
        shared_standin = (
            REPO_ROOT
            / "tests/fixtures/unica_mcp_script_parity/reader-standins/bsl_mcp.py"
        )
        mcp_tool.write_text(
            "import os, runpy\n"
            f"os.environ['UNICA_RLM_CONTRACT_STANDIN_MODE'] = {mode!r}\n"
            f"os.environ['UNICA_RLM_CONTRACT_RPC_LOG'] = {str(request_log)!r}\n"
            f"os.environ['UNICA_RLM_CONTRACT_PID_LOG'] = {str(pid_log)!r}\n"
            f"runpy.run_path({str(shared_standin)!r}, run_name='__main__')\n",
            encoding="utf-8",
        )
        mcp_tool.chmod(mcp_tool.stat().st_mode | 0o755)
        return mcp_tool, index_tool, request_log, index_log, pid_log

    def process_is_alive(self, pid: int) -> bool:
        if os.name == "nt":
            result = subprocess.run(
                ["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
                text=True,
                capture_output=True,
                check=False,
            )
            return result.returncode == 0 and f'"{pid}"' in result.stdout
        result = subprocess.run(
            ["ps", "-o", "stat=", "-p", str(pid)],
            text=True,
            capture_output=True,
            check=False,
        )
        state = result.stdout.strip()
        return result.returncode == 0 and bool(state) and not state.startswith("Z")

    def kill_process_tree_best_effort(self, parent_pid: int, child_pid: int) -> None:
        if os.name == "nt":
            for pid in dict.fromkeys((parent_pid, child_pid)):
                subprocess.run(
                    ["taskkill", "/PID", str(pid), "/T", "/F"],
                    text=True,
                    capture_output=True,
                    check=False,
                )
            return
        try:
            if parent_pid and os.getpgid(parent_pid) == parent_pid:
                os.killpg(parent_pid, signal.SIGKILL)
                return
        except ProcessLookupError:
            pass
        for pid in (child_pid, parent_pid):
            if not pid:
                continue
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def cleanup_process_tree_best_effort(self, parent_pid: int, child_pid: int) -> None:
        if parent_pid or child_pid:
            self.kill_process_tree_best_effort(parent_pid, child_pid)

    def test_rlm_hang_fixture_cleanup_uses_either_known_pid(self) -> None:
        with patch.object(self, "kill_process_tree_best_effort") as kill_tree:
            self.cleanup_process_tree_best_effort(0, 27182)

        kill_tree.assert_called_once_with(0, 27182)

    def test_windows_best_effort_cleanup_targets_surviving_child_independently(
        self,
    ) -> None:
        with patch.object(os, "name", "nt"), patch.object(
            subprocess,
            "run",
        ) as run:
            self.kill_process_tree_best_effort(101, 202)

        self.assertEqual(
            [call.args[0] for call in run.call_args_list],
            [
                ["taskkill", "/PID", "101", "/T", "/F"],
                ["taskkill", "/PID", "202", "/T", "/F"],
            ],
        )

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
                "rlm-bsl-mcp",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_executable(
                tools_dir,
                "v8-runner",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner 0.5.1 version build'\n",
            )

            errors = module.check_tool_contracts(tools_dir)

        self.assertEqual(errors, [])

    def test_rlm_help_forces_utf8_when_stdout_is_captured(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            tool = tools_dir / "rlm-bsl-index.py"
            tool.write_text(
                'print("build: Строить неполный индекс")\n',
                encoding="utf-8",
            )
            with (
                patch.object(
                    module,
                    "TOOL_HELP_CHECKS",
                    [("rlm-bsl-index build", "rlm-bsl-index", [], ["build"])],
                ),
                patch.dict(
                    os.environ,
                    {
                        "LC_ALL": "C",
                        "LANG": "C",
                        "PYTHONCOERCECLOCALE": "0",
                        "PYTHONUTF8": "0",
                        "PYTHONIOENCODING": "ascii",
                    },
                ),
            ):
                errors = module.check_tool_contracts(tools_dir)

        self.assertEqual(errors, [])

    def test_tool_help_failure_includes_bounded_sanitized_diagnostics(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            tool = tools_dir / "rlm-bsl-index.py"
            tool.write_text(
                "from pathlib import Path\n"
                "import sys\n"
                "print(f'OUT-ДИАГНОСТИКА cwd={Path.cwd()} ' + 'x' * 6000)\n"
                "print(f'ERR-ДИАГНОСТИКА cwd={Path.cwd()}', file=sys.stderr)\n"
                "raise SystemExit(7)\n",
                encoding="utf-8",
            )
            with patch.object(
                module,
                "TOOL_HELP_CHECKS",
                [("rlm-bsl-index build", "rlm-bsl-index", ["index", "build"], ["build"])],
            ):
                errors = module.check_tool_contracts(tools_dir)

        self.assertEqual(len(errors), 1)
        error = errors[0]
        self.assertIn("command exited with 7", error)
        self.assertIn("rlm-bsl-index.py index build", error)
        self.assertIn("OUT-ДИАГНОСТИКА", error)
        self.assertIn("ERR-ДИАГНОСТИКА", error)
        self.assertNotIn(str(tools_dir), error)
        self.assertLess(len(error), 5_000)

    def test_tool_help_contracts_do_not_fall_back_to_legacy_rlm_server_name(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            legacy = tools_dir / "rlm-tools-bsl.py"
            legacy.write_text(
                "print('--transport stdio streamable-http')\n",
                encoding="utf-8",
            )
            legacy.chmod(legacy.stat().st_mode | 0o755)

            errors = module.check_tool_contracts(tools_dir)

        self.assertTrue(
            any(
                error.startswith("rlm-bsl-mcp server: binary not found:")
                for error in errors
            ),
            errors,
        )

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
                "rlm-bsl-mcp",
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
                "rlm-bsl-mcp",
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
                "rlm-bsl-mcp",
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
            self.write_executable(tools_dir, "rlm-bsl-mcp", "#!/usr/bin/env sh\nprintf '%s\\n' 'service'\n")
            self.write_executable(tools_dir, "v8-runner", "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner version build'\n")

            errors = module.check_tool_contracts(tools_dir)

        self.assertTrue(any("rlm-bsl-mcp server" in error and "--transport" in error for error in errors), errors)

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

    def test_rlm_mcp_contract_exercises_runtime_helpers_over_json_rpc(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            mcp_tool, index_tool, request_log, index_log, _pid_log = (
                self.write_rlm_contract_standins(root)
            )

            errors = module.check_rlm_mcp_contract(mcp_tool, index_tool)
            requests = [
                json.loads(line)
                for line in request_log.read_text(encoding="utf-8").splitlines()
            ]
            index_call = json.loads(index_log.read_text(encoding="utf-8"))

        self.assertEqual(errors, [])
        self.assertEqual(
            requests[0],
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {"name": "unica-contract", "version": "1"},
                },
            },
        )
        self.assertEqual(
            requests[1],
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
        )
        tool_calls = [
            request["params"]
            for request in requests
            if request.get("method") == "tools/call"
        ]
        self.assertEqual(
            [call["name"] for call in tool_calls],
            ["rlm_start", "rlm_execute", "rlm_execute", "rlm_execute", "rlm_end"],
        )
        start = tool_calls[0]["arguments"]
        self.assertEqual(
            {key: value for key, value in start.items() if key != "path"},
            {
                "query": "ContractTest1",
                "effort": "low",
                "max_output_chars": 100_000,
                "max_execute_calls": 10_000,
                "execution_timeout_seconds": 30,
                "include_metadata": False,
            },
        )
        self.assertEqual(
            [call["arguments"]["code"] for call in tool_calls[1:4]],
            [
                'import json\n_result = search("ContractTest", scope="all", limit=20)\nprint(json.dumps(_result, ensure_ascii=False))',
                'import json\n_result = find_definition("ContractTest1", module_hint=None, limit=20)\nprint(json.dumps(_result, ensure_ascii=False))',
                'import json\n_result = get_object_profile("CommonModule.ContractOne", sections=None, include_flow=False, include_code_usages=False, limit=20)\nprint(json.dumps(_result, ensure_ascii=False))',
            ],
        )
        self.assertNotIn("parse_form", "\n".join(call["arguments"]["code"] for call in tool_calls[1:4]))
        self.assertEqual(index_call["argv"][:2], ["index", "build"])
        self.assertEqual(index_call["argv"][2], start["path"])
        self.assertEqual(
            index_call["index_dir"],
            str(Path(start["path"]).parents[1] / "index"),
        )

    def test_rlm_mcp_contract_confines_upstream_two_ancestor_extension_scan(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            outer = Path(tmp)
            contract_root = outer / "contract"
            contract_root.mkdir()
            outer.joinpath("poison-outer-sibling").touch()
            mcp_tool, index_tool, *_ = self.write_rlm_contract_standins(
                outer,
                index_mode="reject_outer_scan",
            )

            with patch.object(
                module.tempfile,
                "TemporaryDirectory",
                return_value=nullcontext(str(contract_root)),
            ):
                errors = module.check_rlm_mcp_contract(mcp_tool, index_tool)

        self.assertEqual(errors, [])

    def test_rlm_mcp_contract_reports_session_constructor_system_exit(self) -> None:
        module = load_contract_module()

        class Shared:
            class McpSession:
                def __init__(self, *_args, **_kwargs):
                    raise SystemExit("transport constructor failed")

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            mcp_tool, index_tool, *_ = self.write_rlm_contract_standins(root)
            with (
                patch.object(module, "run_rlm_contract_process", return_value=(0, "")),
                patch.object(module, "load_shared_mcp_smoke_module", return_value=Shared),
            ):
                errors = module.check_rlm_mcp_contract(mcp_tool, index_tool)

        self.assertTrue(any("failed to start MCP transport" in error for error in errors), errors)

    def test_rlm_mcp_contract_does_not_label_post_start_failure_as_startup(self) -> None:
        module = load_contract_module()

        class Reader:
            def join(self, timeout):
                pass

            def is_alive(self):
                return False

        class Stream:
            def close(self):
                pass

        class Session:
            def __init__(self, *_args, **_kwargs):
                self.process = type(
                    "Process",
                    (),
                    {"stdin": Stream(), "stdout": Stream(), "stderr": Stream()},
                )()
                self.reader = Reader()
                self.error_reader = Reader()
                self.calls = 0

            def request(self, _payload):
                self.calls += 1
                if self.calls == 1:
                    return {"jsonrpc": "2.0", "id": 1, "result": {}}
                raise RuntimeError("post-start transport failure")

            def notify(self, _payload):
                pass

            def terminate_tree(self, _root):
                pass

        shared = type("Shared", (), {"McpSession": Session})
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            mcp_tool, index_tool, *_ = self.write_rlm_contract_standins(root)
            with (
                patch.object(module, "run_rlm_contract_process", return_value=(0, "")),
                patch.object(module, "load_shared_mcp_smoke_module", return_value=shared),
            ):
                errors = module.check_rlm_mcp_contract(mcp_tool, index_tool)

        self.assertTrue(any("MCP transport failed" in error for error in errors), errors)
        self.assertFalse(any("failed to start" in error for error in errors), errors)

    def test_rlm_mcp_contract_rejects_malformed_helper_payloads(self) -> None:
        module = load_contract_module()
        cases = [
            ("definition_list", "find_definition must return an object"),
            ("string_params", "definitions[].params must be a list"),
            ("string_boolean", "_meta.truncated must be boolean"),
            ("search_object", "search must return a list"),
            ("profile_list", "get_object_profile must return an object"),
            ("profile_error", "get_object_profile returned an error"),
            ("scalar_metadata", "_meta must be an object"),
        ]

        for mode, expected in cases:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                mcp_tool, index_tool, *_ = self.write_rlm_contract_standins(
                    root,
                    mode=mode,
                )
                errors = module.check_rlm_mcp_contract(mcp_tool, index_tool)

                self.assertTrue(any(expected in error for error in errors), errors)
                self.assertFalse(any(str(root) in error for error in errors), errors)

    def test_rlm_mcp_contract_bounds_reads_and_terminates_the_process_tree(self) -> None:
        module = load_contract_module()
        parent_pid = child_pid = 0

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            mcp_tool, index_tool, _request_log, _index_log, pid_log = (
                self.write_rlm_contract_standins(root, mode="hang")
            )
            try:
                with patch.object(module, "RLM_MCP_CONTRACT_TIMEOUT_SECONDS", 0.2):
                    started_at = time.monotonic()
                    errors = module.check_rlm_mcp_contract(mcp_tool, index_tool)
                    elapsed = time.monotonic() - started_at
                parent_pid, child_pid = [
                    int(line)
                    for line in pid_log.read_text(encoding="utf-8").splitlines()
                ]
                deadline = time.monotonic() + 2.0
                while time.monotonic() < deadline and (
                    self.process_is_alive(parent_pid) or self.process_is_alive(child_pid)
                ):
                    time.sleep(0.025)

                self.assertLess(elapsed, 2.0)
                self.assertTrue(any("timed out" in error for error in errors), errors)
                self.assertFalse(self.process_is_alive(parent_pid))
                self.assertFalse(self.process_is_alive(child_pid))
            finally:
                self.cleanup_process_tree_best_effort(parent_pid, child_pid)

    def test_rlm_mcp_contract_bounds_index_build_and_terminates_its_process_tree(self) -> None:
        module = load_contract_module()
        parent_pid = child_pid = 0

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            mcp_tool, index_tool, _request_log, _index_log, pid_log = (
                self.write_rlm_contract_standins(root, index_mode="hang")
            )
            try:
                with patch.object(module, "RLM_MCP_CONTRACT_TIMEOUT_SECONDS", 0.2):
                    started_at = time.monotonic()
                    errors = module.check_rlm_mcp_contract(mcp_tool, index_tool)
                    elapsed = time.monotonic() - started_at
                parent_pid, child_pid = [
                    int(line)
                    for line in pid_log.read_text(encoding="utf-8").splitlines()
                ]
                deadline = time.monotonic() + 2.0
                while time.monotonic() < deadline and (
                    self.process_is_alive(parent_pid) or self.process_is_alive(child_pid)
                ):
                    time.sleep(0.025)

                self.assertLess(elapsed, 2.0)
                self.assertTrue(any("timed out" in error for error in errors), errors)
                self.assertFalse(self.process_is_alive(parent_pid))
                self.assertFalse(self.process_is_alive(child_pid))
            finally:
                self.cleanup_process_tree_best_effort(parent_pid, child_pid)

    def test_rlm_mcp_contract_closes_transport_pipes(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            mcp_tool, index_tool, *_ = self.write_rlm_contract_standins(root)
            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("always", ResourceWarning)
                errors = module.check_rlm_mcp_contract(mcp_tool, index_tool)
                gc.collect()

        self.assertEqual(errors, [])
        self.assertEqual(
            [warning for warning in caught if warning.category is ResourceWarning],
            [],
        )

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
            self.assertEqual(env["RLM_INDEX_DIR"], str(cwd.parents[1] / "index"))
            self.assertEqual(env["RLM_INDEX_SAMPLE_SIZE"], "1000")
            self.assertEqual(env["RLM_INDEX_SAMPLE_THRESHOLD"], "0")
            self.assertEqual(env["RLM_INDEX_SKIP_SAMPLE_HOURS"], "0")
            self.assertEqual(env["PYTHONUTF8"], "1")
            self.assertEqual(env["PYTHONIOENCODING"], "utf-8:surrogateescape")
            return next(outputs)

        with patch.object(
            module,
            "run_rlm_contract_process",
            side_effect=run_rlm,
        ):
            errors = module.check_rlm_mtime_recovery_contract(
                Path("rlm-bsl-index"),
            )

        self.assertEqual(errors, [])
        self.assertEqual(
            actions,
            ["build", "info", "info", "update", "info", "build", "info"],
        )

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
    def test_the_release_checks_every_address_it_publishes(self) -> None:
        """Ядро выпуск сверяет побайтно, поставки — только по адресу.

        Их байты сверены на сборке, когда CI их качал, а адрес после этого не
        трогает никто: опечатка в теге дожила бы до первого вызова движка у
        пользователя. Шаг легко выпасть незамеченным, поэтому он закреплён.
        """
        repo_root = Path(__file__).resolve().parents[2]
        release = (repo_root / ".github/workflows/unica-plugin-release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("scripts/ci/verify-delivery-reachable.py", release)
        self.assertIn("scripts/ci/verify-release-assets.py", release)
        # Выкладывается только то, у чего есть читатель. Подстановочный знак
        # тянул сюда описания поставок: они обещали ассеты, которых на релизе
        # нет, и не читал их никто.
        self.assertIn("dist/runtime/unica-runtime-*.tar.gz", release)
        self.assertIn("dist/runtime/unica-runtime-*.json", release)
        self.assertNotIn("dist/runtime/*-runtime-*", release)
        # Один сквозной прогрев на выпуск: адрес, сумма и раскладка вместе.
        self.assertIn("prefetch --plugin-root", release)
        self.assertTrue(
            (repo_root / "scripts/ci/verify-delivery-reachable.py").is_file()
        )

    def test_both_sides_of_the_wire_approve_the_same_two_origins(self) -> None:
        """Адрес пишет упаковщик, а сверяет bootstrap.

        Разойдись эти списки — выпуск соберётся, а установка откажет уже у
        пользователя: «origin is outside the approved release origin». Ловить
        это надо здесь, а не в поле.
        """
        repo_root = Path(__file__).resolve().parents[2]
        packager = (repo_root / "scripts/ci/package-unica-plugin.py").read_text(
            encoding="utf-8"
        )
        validator = (
            repo_root / "crates/unica-bootstrap/src/manifest.rs"
        ).read_text(encoding="utf-8")

        approved = {
            "https://github.com/IngvarConsulting/unica",
            "https://github.com/IngvarConsulting/unica-toolchain",
        }
        for origin in approved:
            with self.subTest(origin=origin):
                self.assertIn(f'"{origin}"', packager)
                self.assertIn(f'"{origin}/releases/download/"', validator)

        # Список закрыт с обеих сторон: третий адрес — новая запись реестра.
        self.assertEqual(
            len(re.findall(r'"https://github\.com/IngvarConsulting/[\w-]+/releases/download/"', validator)),
            len(approved),
        )


if __name__ == "__main__":
    unittest.main()
