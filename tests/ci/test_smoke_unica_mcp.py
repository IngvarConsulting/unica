from __future__ import annotations

import importlib.util
import json
import os
import socket
import subprocess
import sys
import tempfile
import textwrap
import threading
import time
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SMOKE_SCRIPT = REPO_ROOT / "scripts" / "ci" / "smoke-unica-mcp.py"


def load_module():
    spec = importlib.util.spec_from_file_location("smoke_unica_mcp", SMOKE_SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SmokeUnicaMcpTests(unittest.TestCase):
    def expected_tools(self) -> set[str]:
        module = load_module()
        return module.expected_tool_names(REPO_ROOT / "plugins" / "unica")

    def test_requires_all_logical_source_tools(self) -> None:
        expected = self.expected_tools()

        self.assertTrue(
            {
                "unica.source.resolve",
                "unica.source.children",
                "unica.source.resources",
                "unica.source.read",
            }.issubset(expected)
        )
        # The bounded resource surface is read-only; BSL mutation belongs to
        # unica.code.patch, so the smoke must not demand a writer.
        self.assertNotIn("unica.source.apply", expected)

    def test_waits_for_short_lived_workspace_service_before_temp_cleanup(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            cache_root = Path(directory) / "cache"
            record = cache_root / "services" / "svc-test" / "service.json"
            record.parent.mkdir(parents=True)
            record.write_text("{}\n", encoding="utf-8")

            def retire_service() -> None:
                time.sleep(0.05)
                record.unlink()

            worker = threading.Thread(target=retire_service)
            worker.start()
            try:
                module._wait_for_workspace_services(cache_root, 1.0)
            finally:
                worker.join(timeout=2.0)

            self.assertFalse(worker.is_alive())
            self.assertFalse(record.exists())

    def test_requests_workspace_service_shutdown_before_temp_cleanup(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            cache_root = Path(directory) / "cache"
            record = cache_root / "services" / "svc-test" / "service.json"
            record.parent.mkdir(parents=True)
            listener = socket.socket()
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(1.0)
            record.write_text(
                json.dumps(
                    {
                        "pid": os.getpid(),
                        "port": listener.getsockname()[1],
                        "token": "smoke-secret",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            received: list[dict] = []

            def serve_shutdown() -> None:
                try:
                    connection, _ = listener.accept()
                except (OSError, TimeoutError):
                    return
                with listener, connection:
                    payload = connection.makefile("rb").readline()
                    received.append(json.loads(payload))
                    connection.sendall(
                        b'{"ok":true,"status":"shutdown","shutdown":true}\n'
                    )
                record.unlink()

            worker = threading.Thread(target=serve_shutdown)
            worker.start()
            try:
                service_pids = module._shutdown_workspace_services(cache_root, 1.0)
                module._wait_for_workspace_services(cache_root, 1.0)
            finally:
                listener.close()
                worker.join(timeout=2.0)

            self.assertFalse(worker.is_alive())
            self.assertEqual(service_pids, {os.getpid()})

            self.assertEqual(
                received,
                [
                    {
                        "token": "smoke-secret",
                        "kind": {"type": "shutdown"},
                    }
                ],
            )

    def test_waits_for_workspace_service_process_after_record_is_removed(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            cache_root = Path(directory) / "cache"
            process = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(0.15)"]
            )
            try:
                module._wait_for_workspace_services(
                    cache_root,
                    1.0,
                    {process.pid},
                )
                self.assertFalse(module._process_is_running(process.pid))
            finally:
                if process.poll() is None:
                    process.terminate()
                process.wait(timeout=1.0)

    def test_expected_tools_are_the_canonical_review_ledger_exact_set(self) -> None:
        review = json.loads(
            (REPO_ROOT / "spec/architecture/tool-surface-review.json").read_text(
                encoding="utf-8"
            )
        )

        self.assertEqual(self.expected_tools(), set(review))
        self.assertEqual(
            {name for name in self.expected_tools() if name.startswith("unica.xdto.")},
            {"unica.xdto.info", "unica.xdto.edit"},
        )

    def test_review_ledger_resolution_handles_source_and_packaged_plugin_roots(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            manifest = root / "plugins/unica/.codex-plugin/plugin.json"
            manifest.parent.mkdir(parents=True)
            manifest.write_text("{}\n", encoding="utf-8")
            review_path = root / "spec/architecture/tool-surface-review.json"
            review_path.parent.mkdir(parents=True)
            review_path.write_text(
                json.dumps({"unica.xdto.info": {}, "unica.xdto.edit": {}}),
                encoding="utf-8",
            )
            source_plugin = root / "plugins/unica"
            packaged_plugin = root / ".build/thin/marketplace/plugins/unica"
            source_plugin.mkdir(parents=True, exist_ok=True)
            packaged_plugin.mkdir(parents=True)

            for plugin_root in (source_plugin, packaged_plugin):
                with self.subTest(plugin_root=plugin_root):
                    self.assertEqual(
                        module.expected_tool_names(plugin_root),
                        {"unica.xdto.info", "unica.xdto.edit"},
                    )

    def test_review_ledger_resolution_rejects_ledger_outside_checkout(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            outer = Path(directory)
            unrelated = outer / "spec/architecture/tool-surface-review.json"
            unrelated.parent.mkdir(parents=True)
            unrelated.write_text(
                json.dumps({"unica.source.read": {}}),
                encoding="utf-8",
            )
            checkout = outer / "checkout"
            (checkout / "Cargo.toml").parent.mkdir(parents=True)
            (checkout / "Cargo.toml").write_text(
                "[workspace]\n", encoding="utf-8"
            )
            manifest = checkout / "plugins/unica/.codex-plugin/plugin.json"
            manifest.parent.mkdir(parents=True)
            manifest.write_text("{}\n", encoding="utf-8")
            packaged_plugin = checkout / ".build/thin/marketplace/plugins/unica"
            packaged_plugin.mkdir(parents=True)

            with self.assertRaisesRegex(
                SystemExit,
                "tool-surface-review.json.*checkout root",
            ):
                module.expected_tool_names(packaged_plugin)

    def tool_entries(self, names: set[str] | None = None) -> list[object]:
        module = load_module()
        stable_schemas = json.loads(
            json.dumps(
                {
                    **module.EXPECTED_SOURCE_INPUT_SCHEMAS,
                    **module.EXPECTED_XDTO_INPUT_SCHEMAS,
                }
            )
        )
        selected_names = self.expected_tools() if names is None else names
        return [
            {
                "name": name,
                "inputSchema": stable_schemas.get(
                    name,
                    {
                        "type": "object",
                        "properties": {},
                        "required": [],
                        "additionalProperties": False,
                    },
                ),
                **(
                    {"outputSchema": module.EXPECTED_META_OUTPUT_SCHEMA}
                    if name in module.META_TOOL_NAMES
                    else {}
                ),
            }
            for name in sorted(selected_names)
        ]

    def run_smoke(
        self,
        tool_entries: list[object],
        *,
        server_name: str = "unica",
        instructions: str = "",
        result_drift: bool = False,
        provider_revision: bool = False,
        read_writes: bool = False,
        code_search_status: str = "ok",
        code_search_root_field: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        expected_tools = self.expected_tools()
        module = load_module()
        source_flows = json.loads(
            json.dumps(module.EXPECTED_SOURCE_FLOW_PROJECTIONS)
        )
        if result_drift:
            source_flows["main"]["resolve"]["summary"] = (
                "source.resolve returned a drifted result"
            )
        if provider_revision:
            source_flows["main"]["resolve"]["data"]["candidates"][0][
                "providerRevision"
            ] = "private"
        server_source = textwrap.dedent("""
            import json
            import sys
            from pathlib import Path

            tools = json.loads(r'''__TOOLS__''')
            source_flows = json.loads(r'''__SOURCE_FLOWS__''')
            read_writes = __READ_WRITES__
            code_search_status = __CODE_SEARCH_STATUS__
            code_search_root_field = __CODE_SEARCH_ROOT_FIELD__

            def materialize(value, cwd):
                if isinstance(value, dict):
                    return {
                        key: materialize(child, cwd)
                        for key, child in value.items()
                    }
                if isinstance(value, list):
                    return [materialize(child, cwd) for child in value]
                if value == "<cache-root>":
                    return str(Path(cwd).parent / "cache")
                if value == "<workspace-epoch>":
                    return 1
                return value

            def source_payload(name, args):
                if name in {"unica.source.resolve", "unica.source.children"}:
                    source_set = args["sourceSet"]
                    operation = name.rsplit(".", 1)[1]
                elif name == "unica.source.resources":
                    source_set = args["sourceSet"]
                    operation = "resources"
                else:
                    source_set = args["snapshotId"].rsplit("-", 1)[0]
                    operation = "read"

                payload = materialize(
                    source_flows[source_set][operation],
                    args["cwd"],
                )
                if name == "unica.source.resources":
                    payload["data"]["snapshotId"] = source_set + "-old"
                    payload["data"]["resources"][0]["resourceId"] = (
                        source_set + "-resource-old"
                    )
                elif name == "unica.source.read":
                    payload["data"]["snapshotId"] = args["snapshotId"]
                    payload["data"]["resourceId"] = args["resourceId"]
                    if read_writes:
                        relative = "src" if source_set == "main" else "ext"
                        Path(
                            args["cwd"],
                            relative,
                            "CommonModules/Shared/Ext/Module.bsl",
                        ).write_bytes(b"a read must never write")
                return payload

            def operation_result(ok, summary, diagnostics=None):
                return {
                    "ok": ok,
                    "summary": summary,
                    "changes": [],
                    "warnings": [],
                    "errors": [] if ok else [summary],
                    "artifacts": [],
                    "cache": {
                        "mode": "read", "root": "", "workspace_epoch": 0,
                        "events": [], "invalidated": [], "refreshed": [],
                        "lazy_rebuilt": [], "stale": [], "fresh": [],
                    },
                    **({"diagnostics": diagnostics} if diagnostics is not None else {}),
                }

            for line in sys.stdin:
                message = json.loads(line)
                request_id = message.get("id")
                if message.get("method") == "initialize":
                    initialized = {"jsonrpc": "2.0", "id": request_id, "result": {"serverInfo": {"name": __NAME__}, "instructions": __INSTRUCTIONS__}}
                    # Written as bytes rather than printed: the point of the
                    # UTF-8 case is that real non-ASCII bytes cross the pipe,
                    # and `print` would re-encode them in the console codec.
                    sys.stdout.buffer.write(json.dumps(initialized, ensure_ascii=False).encode("utf-8") + b"\\n")
                    sys.stdout.buffer.flush()
                elif message.get("method") == "tools/list":
                    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": {"tools": tools}}), flush=True)
                elif message.get("method") == "tools/call":
                    name = message["params"]["name"]
                    args = message["params"]["arguments"]
                    if name == "unica.role.info":
                        # ADR-0049 bridge: the stub answers the three
                        # outcomes the smoke asserts and nothing else.
                        if "sourceSet" in args and "RightsPath" in args:
                            payload = operation_result(
                                False,
                                "selector_conflict: unica.role.info accepts either"
                                " `sourceSet` + `metadataPath` or `RightsPath`,"
                                " not both",
                            )
                        elif args.get("metadataPath") == "Role.SmokeRole":
                            payload = operation_result(True, "role inspected")
                            payload["data"] = {"name": "SmokeRole"}
                        else:
                            payload = operation_result(
                                False, "target_not_found: the logical target was not found"
                            )
                        result = {
                            "content": [{"type": "text", "text": json.dumps(payload)}]
                        }
                    elif (
                        name == "unica.source.resolve"
                        and args.get("query") == "Role.SmokeRole"
                    ):
                        payload = operation_result(True, "source.resolve returned 1 canonical candidate(s)")
                        payload["data"] = {
                            "candidates": [
                                {
                                    "displayName": "SmokeRole",
                                    "matchKind": "exact",
                                    "metadataPath": "Role.SmokeRole",
                                    "targetKind": "metadataObject",
                                }
                            ],
                            "completeness": "complete",
                        }
                        result = {
                            "content": [{"type": "text", "text": json.dumps(payload)}]
                        }
                    elif name == "unica.meta.info":
                        if args:
                            payload = operation_result(True, "metadata information inspected")
                        else:
                            payload = operation_result(False, "metadata arguments are invalid", [{"code": "invalid_arguments"}])
                        result = {
                            "content": [{"type": "text", "text": json.dumps(payload)}],
                            "structuredContent": payload,
                            "isError": not payload["ok"],
                        }
                    elif name == "unica.code.search":
                        payload = operation_result(True, "code search completed")
                        bsl_section = {
                            "provider": "bsl-analyzer",
                            "status": code_search_status,
                            "hits": [] if code_search_status != "ok" else [
                                {
                                    "rank": 1,
                                    "path": "CommonModules/Shared/Ext/Module.bsl",
                                    "line": 1,
                                    "symbol": "Run",
                                    "snippet": "Procedure Run()",
                                    "attributes": {},
                                }
                            ],
                            "diagnostics": (
                                ["provider failed"]
                                if code_search_status == "failed"
                                else []
                            ),
                            "artifacts": [],
                        }
                        if code_search_root_field is not None:
                            bsl_section[code_search_root_field] = "private-root"
                        payload["data"] = {
                            "sections": [
                                {
                                    "provider": "rlm", "status": "empty", "hits": [],
                                    "diagnostics": [], "artifacts": [],
                                },
                                bsl_section,
                                {
                                    "provider": "git-grep", "status": "empty", "hits": [],
                                    "diagnostics": [], "artifacts": [],
                                },
                            ]
                        }
                        result = {
                            "content": [{"type": "text", "text": json.dumps(payload)}]
                        }
                    else:
                        payload = source_payload(name, args)
                        result = {"content": [{"type": "text", "text": json.dumps(payload)}]}
                    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}), flush=True)
        """)
        server_source = (
            server_source.replace(
                "__TOOLS__",
                json.dumps(tool_entries, ensure_ascii=False),
            )
            .replace(
                "__SOURCE_FLOWS__",
                json.dumps(source_flows, ensure_ascii=False),
            )
            .replace("__READ_WRITES__", repr(read_writes))
            .replace("__CODE_SEARCH_STATUS__", repr(code_search_status))
            .replace("__CODE_SEARCH_ROOT_FIELD__", repr(code_search_root_field))
            .replace("__NAME__", repr(server_name))
            .replace("__INSTRUCTIONS__", repr(instructions))
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / "server.py"
            server.write_text(server_source, encoding="utf-8")
            review_path = root / "spec/architecture/tool-surface-review.json"
            review_path.parent.mkdir(parents=True)
            review_path.write_text(
                json.dumps({name: {} for name in sorted(expected_tools)}),
                encoding="utf-8",
            )
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            manifest = root / "plugins/unica/.codex-plugin/plugin.json"
            manifest.parent.mkdir(parents=True)
            manifest.write_text("{}\n", encoding="utf-8")
            plugin_root = root / "packaged/plugins/unica"
            plugin_root.mkdir(parents=True)
            return subprocess.run(
                [
                    sys.executable,
                    str(SMOKE_SCRIPT),
                    "--binary",
                    sys.executable,
                    "--binary-arg",
                    str(server),
                    "--plugin-root",
                    str(plugin_root),
                    "--timeout-seconds",
                    "2",
                ],
                capture_output=True,
                text=True,
                check=False,
            )

    def test_accepts_initialize_and_required_tool_responses(self) -> None:
        result = self.run_smoke(self.tool_entries())

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("verified packaged Unica MCP source-resource flow", result.stdout)

    def test_rejects_runtime_missing_a_required_tool(self) -> None:
        result = self.run_smoke(
            self.tool_entries(self.expected_tools() - {"unica.xdto.edit"})
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing", result.stderr)
        self.assertIn("unica.xdto.edit", result.stderr)

    def test_reports_source_tool_missing_from_ledger_before_projection(self) -> None:
        module = load_module()
        expected = self.expected_tools() - {"unica.source.read"}

        with self.assertRaisesRegex(
            SystemExit,
            "source tools.*unica.source.read",
        ):
            module._stable_tool_contract(self.tool_entries(expected), expected)

    def test_rejects_runtime_exposing_an_unexpected_tool(self) -> None:
        result = self.run_smoke(
            self.tool_entries(self.expected_tools() | {"unica.xdto.validate"})
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unexpected", result.stderr)
        self.assertIn("unica.xdto.validate", result.stderr)

    def test_reports_missing_and_unexpected_tools_together(self) -> None:
        tools = self.expected_tools() - {"unica.xdto.edit"}
        tools.add("unica.xdto.validate")
        result = self.run_smoke(self.tool_entries(tools))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing", result.stderr)
        self.assertIn("unica.xdto.edit", result.stderr)
        self.assertIn("unexpected", result.stderr)
        self.assertIn("unica.xdto.validate", result.stderr)

    def test_decodes_mcp_json_as_utf8_independently_of_windows_locale(self) -> None:
        # The server name is fixed by INV-MCP-SERVER-NAME, so it cannot carry
        # the non-ASCII payload this case is about. `instructions` is a
        # documented initialize field and carries it instead, serialized with
        # `ensure_ascii=False` and written as raw UTF-8 bytes.
        result = self.run_smoke(
            self.tool_entries(),
            instructions="Уника читает выгрузку конфигуратора",
        )

        self.assertEqual(result.returncode, 0, result.stderr)

    def test_accepts_the_invariant_server_name(self) -> None:
        result = self.run_smoke(self.tool_entries(), server_name="unica")

        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_a_server_name_other_than_unica(self) -> None:
        # INV-MCP-SERVER-NAME fixes the published identity of the server. A
        # release smoke that accepts any name cannot prove the invariant holds
        # in the artifact it is smoking.
        result = self.run_smoke(self.tool_entries(), server_name="Уника")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("serverInfo", result.stderr)
        self.assertIn("Уника", result.stderr)

    def test_rejects_incomplete_source_schema(self) -> None:
        entries = self.tool_entries()
        source_read = next(
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get("name") == "unica.source.read"
        )
        source_read["inputSchema"]["required"].remove("resourceId")
        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("schema", result.stderr)

    def test_rejects_missing_meta_output_schema(self) -> None:
        entries = self.tool_entries()
        meta_info = next(
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get("name") == "unica.meta.info"
        )
        meta_info.pop("outputSchema")

        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Meta output schema", result.stderr)
        self.assertIn("unica.meta.info", result.stderr)

    def test_rejects_output_schema_on_non_meta_tool(self) -> None:
        entries = self.tool_entries()
        project_status = next(
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get("name") == "unica.project.status"
        )
        project_status["outputSchema"] = load_module().EXPECTED_META_OUTPUT_SCHEMA

        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("non-Meta tool", result.stderr)
        self.assertIn("unica.project.status", result.stderr)

    def test_rejects_xdto_info_schema_missing_required_target(self) -> None:
        entries = self.tool_entries()
        xdto_info = next(
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get("name") == "unica.xdto.info"
        )
        xdto_info["inputSchema"]["required"].remove("metadataPath")

        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("XDTO input schema", result.stderr)

    def test_rejects_xdto_edit_schema_missing_operation_branch_requirement(self) -> None:
        entries = self.tool_entries()
        xdto_edit = next(
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get("name") == "unica.xdto.edit"
        )
        add_value_type = next(
            branch
            for branch in xdto_edit["inputSchema"]["oneOf"]
            if branch["properties"]["operation"]["const"] == "add-value-type"
        )
        add_value_type["required"].remove("base")

        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("XDTO input schema", result.stderr)

    def test_rejects_expected_xdto_tool_without_input_schema(self) -> None:
        entries = self.tool_entries()
        xdto_info = next(
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get("name") == "unica.xdto.info"
        )
        xdto_info.pop("inputSchema")

        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("input schema", result.stderr)
        self.assertIn("unica.xdto.info", result.stderr)

    def test_rejects_expected_xdto_tool_with_non_object_input_schema(self) -> None:
        entries = self.tool_entries()
        xdto_edit = next(
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get("name") == "unica.xdto.edit"
        )
        xdto_edit["inputSchema"] = []

        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("input schema", result.stderr)
        self.assertIn("unica.xdto.edit", result.stderr)

    def test_rejects_expected_xdto_schema_not_declaring_an_object(self) -> None:
        entries = self.tool_entries()
        xdto_edit = next(
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get("name") == "unica.xdto.edit"
        )
        xdto_edit["inputSchema"] = {
            "type": "array",
            "properties": {},
            "required": [],
            "additionalProperties": False,
        }

        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("type object", result.stderr)
        self.assertIn("unica.xdto.edit", result.stderr)

    def test_rejects_duplicate_expected_tool_name(self) -> None:
        entries = self.tool_entries()
        duplicate = next(
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get("name") == "unica.xdto.info"
        )
        entries.append(json.loads(json.dumps(duplicate)))

        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("duplicate", result.stderr)
        self.assertIn("unica.xdto.info", result.stderr)

    def test_rejects_malformed_non_object_tool_entry(self) -> None:
        entries = self.tool_entries()
        entries.append("not-a-tool")

        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("malformed entries", result.stderr)

    def test_rejects_empty_tool_name_as_malformed(self) -> None:
        entries = self.tool_entries()
        entries.append(
            {
                "name": "",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": False,
                },
            }
        )

        result = self.run_smoke(entries)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("malformed entries", result.stderr)

    def test_rejects_stable_source_result_drift(self) -> None:
        result = self.run_smoke(self.tool_entries(), result_drift=True)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stable", result.stderr)

    def test_rejects_provider_revision_leakage(self) -> None:
        result = self.run_smoke(
            self.tool_entries(),
            provider_revision=True,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("providerRevision", result.stderr)

    def test_rejects_a_read_that_writes(self) -> None:
        """The whole source surface is read-only, so any byte it changes fails."""
        result = self.run_smoke(self.tool_entries(), read_writes=True)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("read-only", result.stderr)

    def test_rejects_failed_bsl_analyzer_section_in_packaged_search(self) -> None:
        result = self.run_smoke(
            self.tool_entries(),
            code_search_status="failed",
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("bsl-analyzer", result.stderr)

    def test_retries_bsl_analyzer_while_search_index_is_being_built(self) -> None:
        payload = {
            "data": {
                "sections": [
                    {
                        "provider": "rlm",
                        "status": "empty",
                        "hits": [],
                        "diagnostics": [],
                        "artifacts": [],
                    },
                    {
                        "provider": "bsl-analyzer",
                        "status": "unavailable",
                        "hits": [],
                        "diagnostics": [
                            "Search index is being built, please try again in a moment."
                        ],
                        "artifacts": [],
                    },
                    {
                        "provider": "git-grep",
                        "status": "empty",
                        "hits": [],
                        "diagnostics": [],
                        "artifacts": [],
                    },
                ]
            }
        }

        self.assertFalse(load_module()._bsl_search_is_ready(payload))

    def test_rejects_upstream_root_identity_leaking_from_packaged_search(self) -> None:
        result = self.run_smoke(
            self.tool_entries(),
            code_search_root_field="rootId",
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("rootId", result.stderr)


if __name__ == "__main__":
    unittest.main()
