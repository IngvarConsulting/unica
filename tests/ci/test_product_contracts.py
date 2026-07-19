from __future__ import annotations

import importlib.util
import json
import re
import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path


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

    def test_ai_entrypoints_document_source_of_truth_and_ignored_corpus(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        entrypoint = repo_root / "AGENTS.md"

        text = entrypoint.read_text(encoding="utf-8")

        self.assertIn("code/tests/package metadata > spec > historical plans", text)
        for ignored in ["docs/research", "docs/its", "target", ".build", "dist"]:
            with self.subTest(ignored=ignored):
                self.assertIn(ignored, text)

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

    def test_readme_documents_the_frozen_v076_bridge(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        readme = (repo_root / "README.md").read_text(encoding="utf-8")

        self.assertIn("| Исходное состояние |", readme)
        self.assertIn(
            "releases/download/v0.7.6/install-unica.sh",
            readme,
        )
        self.assertIn(
            "releases/download/v0.7.6/install-unica.ps1",
            readme,
        )
        self.assertIn("v0.7.5", readme)
        self.assertIn("техническ", readme.lower())
        self.assertIn("v0.8.0", readme)

    def test_v076_release_note_declares_the_legacy_boundary(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        note = (repo_root / "docs/releases/v0.7.6.md").read_text(encoding="utf-8")

        for value in ("#90", "unica-local", "v0.7.6", "v0.8.0", "manual"):
            with self.subTest(value=value):
                self.assertIn(value, note)

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

        release_notes = [
            repo_root / "docs/releases/v0.7.1.md",
            repo_root / "docs/releases/v0.7.2.md",
            repo_root / "docs/releases/v0.7.5.md",
            repo_root / "docs/releases/v0.7.6.md",
        ]
        fat_archive_mentions = [
            str(path.relative_to(repo_root))
            for path in release_notes
            if "unica-codex-marketplace-" in path.read_text(encoding="utf-8")
        ]
        self.assertEqual(fat_archive_mentions, [])

    def test_superpowers_plans_are_marked_historical(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        plan_dir = repo_root / "docs" / "superpowers" / "plans"

        for plan in plan_dir.glob("*.md"):
            with self.subTest(plan=plan.name):
                head = "\n".join(plan.read_text(encoding="utf-8").splitlines()[:8])
                self.assertIn("Historical", head)

    def test_script_backed_skill_exceptions_are_documented_by_adr(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        adr = repo_root / "spec" / "decisions" / "0007-script-backed-utility-skill-exceptions.md"

        text = adr.read_text(encoding="utf-8")

        self.assertIn("web-test", text)
        self.assertIn("img-grid", text)
        self.assertIn("permanent local-tool exception", text)

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
