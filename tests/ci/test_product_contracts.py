from __future__ import annotations

from contextlib import closing
import importlib.util
import sqlite3
import tempfile
import unittest
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
    def write_script(self, scripts_dir: Path, name: str, body: str) -> None:
        path = scripts_dir / name
        path.write_text(body, encoding="utf-8")
        path.chmod(path.stat().st_mode | 0o755)

    def test_tool_help_contracts_pass_with_expected_cli_surface(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            scripts_dir = Path(tmp)
            self.write_script(
                scripts_dir,
                "run-bsl-analyzer.sh",
                "#!/usr/bin/env sh\n"
                "printf '%s\\n' '--source-dir --format jsonl baseline --profile workspace reference "
                "--mode stdio --scenarios --json mcp serve analyze search smoke'\n",
            )
            self.write_script(
                scripts_dir,
                "run-rlm-bsl-index.sh",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n",
            )
            self.write_script(
                scripts_dir,
                "run-rlm-tools-bsl.sh",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_script(
                scripts_dir,
                "run-v8-runner.sh",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner 0.5.1 version build'\n",
            )

            errors = module.check_tool_contracts(scripts_dir)

        self.assertEqual(errors, [])

    def test_tool_help_contracts_accept_relative_scripts_dir(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory(dir=Path.cwd()) as tmp:
            scripts_dir = Path(tmp)
            self.write_script(
                scripts_dir,
                "run-bsl-analyzer.sh",
                "#!/usr/bin/env sh\n"
                "printf '%s\\n' '--source-dir --format jsonl baseline --profile workspace reference "
                "--mode stdio --scenarios --json mcp serve analyze search smoke'\n",
            )
            self.write_script(
                scripts_dir,
                "run-rlm-bsl-index.sh",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n",
            )
            self.write_script(
                scripts_dir,
                "run-rlm-tools-bsl.sh",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_script(
                scripts_dir,
                "run-v8-runner.sh",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner 0.5.1 version build'\n",
            )

            errors = module.check_tool_contracts(scripts_dir.relative_to(Path.cwd()))

        self.assertEqual(errors, [])

    def test_tool_help_contracts_report_missing_expected_flag(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            scripts_dir = Path(tmp)
            self.write_script(scripts_dir, "run-bsl-analyzer.sh", "#!/usr/bin/env sh\nprintf '%s\\n' 'analyze'\n")
            self.write_script(scripts_dir, "run-rlm-bsl-index.sh", "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n")
            self.write_script(
                scripts_dir,
                "run-rlm-tools-bsl.sh",
                "#!/usr/bin/env sh\nprintf '%s\\n' '--transport stdio streamable-http service'\n",
            )
            self.write_script(scripts_dir, "run-v8-runner.sh", "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner version build'\n")

            errors = module.check_tool_contracts(scripts_dir)

        self.assertTrue(any("--source-dir" in error for error in errors), errors)

    def test_tool_help_contracts_report_missing_rlm_server_transport_surface(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            scripts_dir = Path(tmp)
            self.write_script(
                scripts_dir,
                "run-bsl-analyzer.sh",
                "#!/usr/bin/env sh\n"
                "printf '%s\\n' '--source-dir --format jsonl baseline --profile workspace reference "
                "--mode stdio --scenarios --json mcp serve analyze search smoke'\n",
            )
            self.write_script(scripts_dir, "run-rlm-bsl-index.sh", "#!/usr/bin/env sh\nprintf '%s\\n' 'index build update info'\n")
            self.write_script(scripts_dir, "run-rlm-tools-bsl.sh", "#!/usr/bin/env sh\nprintf '%s\\n' 'service'\n")
            self.write_script(scripts_dir, "run-v8-runner.sh", "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner version build'\n")

            errors = module.check_tool_contracts(scripts_dir)

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
