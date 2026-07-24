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
    BSL_ANALYZER_HELP = (
        "#!/usr/bin/env sh\n"
        "case \"$*\" in\n"
        "  'analyze --help') printf '%s\\n' '--source-dir --format jsonl' ;;\n"
        "  'mcp serve --help') printf '%s\\n' '--profile --source-dir --mode stdio' ;;\n"
        "  *) exit 1 ;;\n"
        "esac\n"
    )

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

    def test_rlm_mtime_recovery_contract_checks_real_cli_sequence(self) -> None:
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
            actions.append(command[2])
            self.assertEqual(cwd, Path(command[3]))
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
        for command in git_commands[:5]:
            self.assertEqual(command[:5], signing_disabled)

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
