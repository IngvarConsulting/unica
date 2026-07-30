from __future__ import annotations

import importlib.util
import json
import os
import queue
import subprocess
import tempfile
import threading
import time
import unittest
from contextlib import contextmanager
from pathlib import Path


MCP_HANDSHAKE_ID = "unica-ci-handshake"
MCP_INITIALIZE_PARAMS = {
    "protocolVersion": "2025-06-18",
    "capabilities": {},
    "clientInfo": {"name": "unica-ci", "version": "1"},
}
MCP_HANDSHAKE = [
    {
        "jsonrpc": "2.0",
        "id": MCP_HANDSHAKE_ID,
        "method": "initialize",
        "params": MCP_INITIALIZE_PARAMS,
    },
    {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
]


def source_smoke_oracle():
    script = Path(__file__).resolve().parents[2] / "scripts/ci/smoke-unica-mcp.py"
    spec = importlib.util.spec_from_file_location("smoke_unica_mcp_oracle", script)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def assert_no_physical_source_selectors(value: object) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in {"path", "sourceDir", "provider", "providerId", "providerRevision", "handle"}:
                raise AssertionError(f"source response leaked physical selector {key}")
            assert_no_physical_source_selectors(child)
    elif isinstance(value, list):
        for child in value:
            assert_no_physical_source_selectors(child)


def snapshot_workspace_files(root: Path) -> dict[str, bytes]:
    return {
        path.relative_to(root).as_posix(): path.read_bytes()
        for path in root.rglob("*")
        if path.is_file()
    }


class UnicaMcpSmokeTests(unittest.TestCase):
    def repo_root(self) -> Path:
        return Path(__file__).resolve().parents[2]

    def call_mcp(self, messages: list[dict], *, cache_dir: Path | None = None) -> list[dict]:
        env = os.environ.copy()
        if cache_dir is not None:
            env["UNICA_CACHE_DIR"] = str(cache_dir)
        process = subprocess.Popen(
            ["cargo", "run", "--quiet", "--bin", "unica", "--"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=self.repo_root(),
            env=env,
        )
        assert process.stdin is not None
        assert process.stdout is not None
        assert process.stderr is not None
        deadline = time.monotonic() + 30
        lines: queue.Queue[str] = queue.Queue()

        def read_stdout() -> None:
            while True:
                line = process.stdout.readline()
                lines.put(line)
                if not line:
                    return

        reader = threading.Thread(target=read_stdout, daemon=True)
        reader.start()
        try:
            # The rmcp-based server requires the MCP handshake first; prepend it
            # unless the scenario drives initialize itself.
            injected_handshake = messages and messages[0].get("method") != "initialize"
            if injected_handshake:
                messages = MCP_HANDSHAKE + messages
            for message in messages:
                process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()

            expected_responses = sum("id" in message for message in messages)
            responses = []
            for _ in range(expected_responses):
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    self.fail("timed out waiting for MCP response")
                try:
                    line = lines.get(timeout=remaining)
                except queue.Empty:
                    self.fail("timed out waiting for MCP response")
                if not line:
                    self.fail("MCP process exited before all responses arrived")
                responses.append(json.loads(line))

            process.stdin.close()
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    self.fail("timed out waiting for MCP stdout EOF")
                try:
                    trailing = lines.get(timeout=remaining)
                except queue.Empty:
                    self.fail("timed out waiting for MCP stdout EOF")
                if not trailing:
                    break
                self.fail(f"unexpected MCP response after expected ids: {trailing.strip()}")
            return_code = process.wait(timeout=max(0.1, deadline - time.monotonic()))
            stderr = process.stderr.read()
            self.assertEqual(return_code, 0, stderr)
            return [r for r in responses if not injected_handshake or r.get("id") != MCP_HANDSHAKE_ID]
        finally:
            if not process.stdin.closed:
                process.stdin.close()
            if process.poll() is None:
                process.kill()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
            process.stdout.close()
            process.stderr.close()

    @contextmanager
    def mcp_session(self, *, cache_dir: Path | None = None):
        env = os.environ.copy()
        if cache_dir is not None:
            env["UNICA_CACHE_DIR"] = str(cache_dir)
        process = subprocess.Popen(
            ["cargo", "run", "--quiet", "--bin", "unica", "--"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=self.repo_root(),
            env=env,
        )
        assert process.stdin is not None
        assert process.stdout is not None
        assert process.stderr is not None
        lines: queue.Queue[str] = queue.Queue()

        def read_stdout() -> None:
            for line in process.stdout:
                lines.put(line)
            lines.put("")

        threading.Thread(target=read_stdout, daemon=True).start()

        # The session outlives many calls, so stderr has to be drained too:
        # a full pipe buffer would stall the server and read as a timeout.
        diagnostics: list[str] = []

        def read_stderr() -> None:
            for line in process.stderr:
                diagnostics.append(line)

        threading.Thread(target=read_stderr, daemon=True).start()

        def request(message: dict) -> dict:
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()
            try:
                line = lines.get(timeout=30)
            except queue.Empty:
                self.fail(
                    "timed out waiting for interactive MCP response: "
                    + "".join(diagnostics)
                )
            if not line:
                self.fail(
                    "interactive MCP process exited before a response: "
                    + "".join(diagnostics)
                )
            response = json.loads(line)
            self.assertEqual(response.get("id"), message.get("id"), response)
            return response

        try:
            response = request({**MCP_HANDSHAKE[0], "id": 1})
            self.assertIn("result", response)
            process.stdin.write(json.dumps(MCP_HANDSHAKE[1]) + "\n")
            process.stdin.flush()
            yield request
        finally:
            if not process.stdin.closed:
                process.stdin.close()
            try:
                try:
                    return_code = process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    process.kill()
                    # Reap the killed child, otherwise it is left a zombie.
                    process.wait()
                    self.fail("interactive MCP process did not exit")
                self.assertEqual(return_code, 0, "".join(diagnostics))
            finally:
                # Closed on every exit path, including a failed assertion.
                for stream in (process.stdout, process.stderr):
                    if stream is not None and not stream.closed:
                        stream.close()

    def source_fixture(self, root: Path) -> dict[str, bytes]:
        (root / "src/CommonModules/Shared/Ext").mkdir(parents=True)
        (root / "ext/CommonModules/Shared/Ext").mkdir(parents=True)
        (root / "v8project.yaml").write_text(
            "format: DESIGNER\nsource-set:\n"
            "  - name: main\n    type: CONFIGURATION\n    path: src\n"
            "  - name: extension\n    type: EXTENSION\n    path: ext\n",
            encoding="utf-8",
        )
        xml = {}
        for source_set, name, method in [("src", "Main", "Run"), ("ext", "Extension", "RunExtension")]:
            descriptor = (
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">"
                f"<Configuration><Properties><Name>{name}</Name>"
                + ("<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>" if source_set == "ext" else "")
                + "</Properties><ChildObjects><CommonModule>Shared</CommonModule>"
                "</ChildObjects></Configuration></MetaDataObject>"
            ).encode()
            (root / source_set / "Configuration.xml").write_bytes(descriptor)
            module_descriptor = (
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">"
                "<CommonModule><Properties><Name>Shared</Name></Properties></CommonModule>"
                "</MetaDataObject>"
            ).encode()
            (root / source_set / "CommonModules/Shared.xml").write_bytes(module_descriptor)
            module_path = root / source_set / "CommonModules/Shared/Ext/Module.bsl"
            module_path.write_bytes((f"\ufeffProcedure {method}()\r\nEndProcedure\r\n").encode())
            xml[str(root / source_set / "Configuration.xml")] = descriptor
            xml[str(root / source_set / "CommonModules/Shared.xml")] = module_descriptor
        return xml

    def test_initialize_lists_single_unica_server(self) -> None:
        responses = self.call_mcp(
            [
                {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": MCP_INITIALIZE_PARAMS},
                {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            ]
        )

        self.assertEqual(responses[0]["result"]["serverInfo"]["name"], "unica")
        tools = {tool["name"] for tool in responses[1]["result"]["tools"]}
        self.assertIn("unica.project.status", tools)
        self.assertIn("unica.project.map", tools)
        self.assertIn("unica.form.edit", tools)
        self.assertIn("unica.epf.init", tools)
        self.assertIn("unica.erf.init", tools)
        self.assertIn("unica.build.load", tools)
        self.assertIn("unica.runtime.execute", tools)
        self.assertIn("unica.standards.explain", tools)

    def test_source_resources_cover_configuration_and_extension_through_one_jsonrpc_session(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            temp = Path(tmp)
            root = temp / "workspace"
            self.source_fixture(root)
            before = snapshot_workspace_files(root)
            oracle = source_smoke_oracle()
            cache_root = temp / "cache"
            with self.mcp_session(cache_dir=cache_root) as request:
                next_id = 2

                def tool(name: str, arguments: dict) -> dict:
                    nonlocal next_id
                    response = request(
                        {
                            "jsonrpc": "2.0",
                            "id": next_id,
                            "method": "tools/call",
                            "params": {"name": name, "arguments": arguments},
                        }
                    )
                    next_id += 1
                    self.assertNotIn("error", response, response)
                    payload = json.loads(response["result"]["content"][0]["text"])
                    self.assertTrue(payload["ok"], payload)
                    assert_no_physical_source_selectors(payload)
                    return payload

                listed = request({"jsonrpc": "2.0", "id": next_id, "method": "tools/list", "params": {}})
                next_id += 1
                oracle._stable_tool_contract(listed["result"]["tools"])
                tools = {item["name"]: item for item in listed["result"]["tools"]}
                source_tools = {
                    "unica.source.resolve",
                    "unica.source.children",
                    "unica.source.resources",
                    "unica.source.read",
                    "unica.source.apply",
                }
                self.assertTrue(source_tools.issubset(tools))
                for name in source_tools:
                    schema = tools[name]["inputSchema"]
                    self.assertFalse({"path", "sourceDir", "provider", "handle"} & set(schema["properties"]))
                    assert_no_physical_source_selectors(schema)

                for source_set in ("main", "extension"):
                    target = "CommonModule.Shared.Module"
                    resolve = tool(
                        "unica.source.resolve",
                        {"cwd": str(root), "sourceSet": source_set, "query": target, "mode": "exact", "targetKind": "module"},
                    )
                    self.assertEqual([candidate["metadataPath"] for candidate in resolve["data"]["candidates"]], [target])
                    children = tool(
                        "unica.source.children",
                        {"cwd": str(root), "sourceSet": source_set, "metadataPath": "CommonModule.Shared"},
                    )
                    self.assertIn(target, [child["metadataPath"] for child in children["data"]["children"]])
                    resources = tool(
                        "unica.source.resources",
                        {"cwd": str(root), "sourceSet": source_set, "metadataPath": target, "scope": "self"},
                    )
                    self.assertEqual(resources["cache"]["events"], [])
                    self.assertEqual(resources["cache"]["invalidated"], [])
                    resource = resources["data"]["resources"][0]
                    read = tool(
                        "unica.source.read",
                        {"cwd": str(root), "snapshotId": resources["data"]["snapshotId"], "resourceId": resource["resourceId"]},
                    )
                    self.assertEqual(read["data"]["textProfile"]["bomPrefixBytes"], 3)
                    self.assertEqual(read["data"]["textProfile"]["eol"], "crlf")
                    content = "Procedure Changed()\nEndProcedure\n"
                    apply_args = {
                        "cwd": str(root), "snapshotId": resources["data"]["snapshotId"], "resourceId": resource["resourceId"],
                        "expectedHash": resource["hash"], "content": content, "contentEncoding": "utf-8",
                    }
                    before_preview = snapshot_workspace_files(root)
                    preview = tool("unica.source.apply", apply_args)
                    self.assertEqual(
                        snapshot_workspace_files(root),
                        before_preview,
                        f"{source_set} preview changed workspace bytes",
                    )
                    expected_after_apply = dict(before_preview)
                    source_root = "src" if source_set == "main" else "ext"
                    module_path = (
                        f"{source_root}/CommonModules/Shared/Ext/Module.bsl"
                    )
                    expected_after_apply[module_path] = (
                        b"\xef\xbb\xbfProcedure Changed()\r\nEndProcedure\r\n"
                    )
                    applied = tool("unica.source.apply", {**apply_args, "dryRun": False})
                    self.assertEqual(
                        snapshot_workspace_files(root),
                        expected_after_apply,
                        f"{source_set} apply changed an unexpected workspace byte",
                    )
                    self.assertEqual(preview["data"]["postHash"], applied["data"]["postHash"])
                    self.assertEqual(preview["cache"]["mode"], "dry-run")
                    self.assertEqual(applied["cache"]["mode"], "applied")
                    self.assertEqual(preview["cache"]["events"], ["SourceResourcesReplaced"])
                    self.assertEqual(applied["cache"]["events"], ["SourceResourcesReplaced"])
                    self.assertEqual(preview["cache"]["invalidated"], ["bsl_diagnostics", "bsl_index"])
                    self.assertEqual(applied["cache"]["invalidated"], ["bsl_diagnostics", "bsl_index"])
                    current = tool(
                        "unica.source.resources",
                        {"cwd": str(root), "sourceSet": source_set, "metadataPath": target, "scope": "self"},
                    )
                    self.assertNotEqual(
                        current["data"]["snapshotId"],
                        resources["data"]["snapshotId"],
                        f"{source_set} postimage reused the preimage snapshot",
                    )
                    current_resource = current["data"]["resources"][0]
                    postimage = tool(
                        "unica.source.read",
                        {"cwd": str(root), "snapshotId": current["data"]["snapshotId"], "resourceId": current_resource["resourceId"]},
                    )
                    self.assertEqual(postimage["data"]["content"], "\ufeffProcedure Changed()\r\nEndProcedure\r\n")
                    self.assertEqual(
                        oracle.source_flow_projection(
                            source_set,
                            cache_root,
                            resolve,
                            children,
                            resources,
                            read,
                            preview,
                            applied,
                            current,
                            postimage,
                        ),
                        oracle.expected_source_flow_projection(source_set),
                    )

            after = snapshot_workspace_files(root)
            expected = dict(before)
            expected_module = (
                b"\xef\xbb\xbfProcedure Changed()\r\nEndProcedure\r\n"
            )
            expected["src/CommonModules/Shared/Ext/Module.bsl"] = expected_module
            expected["ext/CommonModules/Shared/Ext/Module.bsl"] = expected_module
            self.assertEqual(after, expected)

    def test_source_transport_rejects_legacy_patch_selectors_and_descriptor_replacement(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            temp = Path(tmp)
            root = temp / "workspace"
            self.source_fixture(root)
            with self.mcp_session(cache_dir=temp / "cache") as request:
                next_id = 2

                def raw_tool(name: str, arguments: dict) -> dict:
                    nonlocal next_id
                    response = request(
                        {
                            "jsonrpc": "2.0",
                            "id": next_id,
                            "method": "tools/call",
                            "params": {"name": name, "arguments": arguments},
                        }
                    )
                    next_id += 1
                    return response

                def assert_rpc_error(
                    response: dict,
                    *,
                    prefix: str,
                    hint: str,
                ) -> None:
                    expected_message = f"{prefix}: {hint}"
                    self.assertEqual(response["jsonrpc"], "2.0")
                    self.assertIsInstance(response["id"], int)
                    self.assertNotIn("result", response)
                    self.assertEqual(
                        response["error"],
                        {"code": -32000, "message": expected_message},
                    )
                    actual_prefix, separator, actual_hint = response[
                        "error"
                    ]["message"].partition(": ")
                    self.assertEqual(separator, ": ")
                    self.assertEqual(actual_prefix, prefix)
                    self.assertEqual(actual_hint, hint)

                patch_args = {
                    "cwd": str(root), "sourceSet": "main", "metadataPath": "CommonModule.Shared.Module",
                    "operation": "insert", "selector": {"method": "Run"}, "content": "Procedure Added()\nEndProcedure",
                }
                legacy_hint = (
                    "unica.code.patch no longer accepts `path` or "
                    "`sourceDir`; use `sourceSet + metadataPath`"
                )
                for legacy in ({"path": "src/CommonModules/Shared/Ext/Module.bsl"}, {"sourceDir": "src"}, {"path": "src/CommonModules/Shared/Ext/Module.bsl", "sourceDir": "src"}):
                    result = raw_tool("unica.code.patch", {**patch_args, **legacy})
                    assert_rpc_error(
                        result,
                        prefix="legacy_target_removed",
                        hint=legacy_hint,
                    )

                resources_response = raw_tool(
                    "unica.source.resources",
                    {"cwd": str(root), "sourceSet": "main", "scope": "self"},
                )
                self.assertNotIn("error", resources_response)
                payload = json.loads(
                    resources_response["result"]["content"][0]["text"]
                )
                self.assertEqual(payload["data"]["completeness"], "complete")
                descriptors = [item for item in payload["data"]["resources"] if item["role"] == "configurationDescriptor"]
                self.assertEqual(len(descriptors), 1, payload)
                descriptor = descriptors[0]
                result = raw_tool(
                    "unica.source.apply",
                    {
                        "cwd": str(root), "snapshotId": payload["data"]["snapshotId"],
                        "resourceId": descriptor["resourceId"], "expectedHash": descriptor["hash"],
                        "content": "<replacement/>", "contentEncoding": "utf-8", "dryRun": False,
                    },
                )
                assert_rpc_error(
                    result,
                    prefix="resource_not_replaceable",
                    hint="the snapshotted resource role is not replaceable",
                )
                partial_response = raw_tool(
                    "unica.source.resources",
                    {"cwd": str(root), "sourceSet": "main", "scope": "aggregate", "limit": 1},
                )
                self.assertNotIn("error", partial_response)
                partial_payload = json.loads(
                    partial_response["result"]["content"][0]["text"]
                )
                self.assertEqual(partial_payload["data"]["completeness"], "partial")
                partial_resource = partial_payload["data"]["resources"][0]
                partial_apply = raw_tool(
                    "unica.source.apply",
                    {
                        "cwd": str(root), "snapshotId": partial_payload["data"]["snapshotId"],
                        "resourceId": partial_resource["resourceId"], "expectedHash": partial_resource["hash"],
                        "content": "<replacement/>", "contentEncoding": "utf-8", "dryRun": False,
                    },
                )
                assert_rpc_error(
                    partial_apply,
                    prefix="snapshot_incomplete",
                    hint="source.apply requires a complete resource snapshot",
                )

    def test_notifications_do_not_count_as_responses(self) -> None:
        responses = self.call_mcp(
            [
                {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": MCP_INITIALIZE_PARAMS},
                {"jsonrpc": "2.0", "method": "notifications/initialized"},
                {
                    "jsonrpc": "2.0",
                    "method": "notifications/cancelled",
                    "params": {"requestId": "already-complete", "reason": "smoke"},
                },
                {"jsonrpc": "2.0", "id": 2, "method": "ping"},
            ]
        )

        self.assertEqual([response["id"] for response in responses], [1, 2])

    def test_mutating_dry_run_reports_cache_impact(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            responses = self.call_mcp(
                [
                    {
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.form.edit",
                            "arguments": {"dryRun": True, "cwd": str(tmp_path)},
                        },
                    }
                ],
                cache_dir=tmp_path / "cache",
            )

        text = responses[0]["result"]["content"][0]["text"]
        payload = json.loads(text)
        self.assertTrue(payload["ok"])
        self.assertIn("cache", payload)
        self.assertEqual(payload["cache"]["mode"], "dry-run")
        self.assertIn("FormChanged", payload["cache"]["events"])
        self.assertIn("metadata_graph", payload["cache"]["invalidated"])

    def test_runtime_execute_dry_run_reports_runner_cache_impact(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            responses = self.call_mcp(
                [
                    {
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.runtime.execute",
                            "arguments": {
                                "cwd": str(tmp_path),
                                "operation": "dump",
                            },
                        },
                    }
                ],
                cache_dir=tmp_path / "cache",
            )

        text = responses[0]["result"]["content"][0]["text"]
        payload = json.loads(text)
        self.assertTrue(payload["ok"])
        self.assertEqual(payload["cache"]["mode"], "dry-run")
        self.assertIn("SourceSetChanged", payload["cache"]["events"])
        command = " ".join(payload["command"]).replace("\\", "/")
        self.assertIn("bin/", command)
        self.assertIn("v8-runner", command)
        self.assertNotIn("run-v8-runner.sh", command)

    def test_external_init_creates_epf_and_erf_fixture_scenarios(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            (tmp_path / "v8project.yaml").write_text(
                "format: DESIGNER\n"
                "source-set:\n"
                "  - name: external-processors\n"
                "    type: EXTERNAL_DATA_PROCESSORS\n"
                "    path: epf\n"
                "  - name: external-reports\n"
                "    type: EXTERNAL_REPORTS\n"
                "    path: erf\n",
                encoding="utf-8",
            )
            responses = self.call_mcp(
                [
                    {
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.epf.init",
                            "arguments": {
                                "cwd": str(tmp_path),
                                "Name": "Import",
                                "Synonym": "Import & prices",
                                "OutputDir": "epf",
                                "FormName": "MainForm",
                                "dryRun": False,
                            },
                        },
                    },
                    {
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.erf.init",
                            "arguments": {
                                "cwd": str(tmp_path),
                                "Name": "Balances",
                                "OutputDir": "erf",
                                "dryRun": False,
                            },
                        },
                    },
                    {
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.project.map",
                            "arguments": {"cwd": str(tmp_path)},
                        },
                    },
                ],
                cache_dir=tmp_path / "cache",
            )

            payloads = {
                response["id"]: json.loads(response["result"]["content"][0]["text"])
                for response in responses
            }
            self.assertTrue(payloads[1]["ok"], payloads[1])
            self.assertTrue(payloads[2]["ok"], payloads[2])
            self.assertEqual(len(payloads[1]["artifacts"]), 5)
            self.assertEqual(len(payloads[2]["artifacts"]), 2)

            epf_descriptor = (tmp_path / "epf/Import.xml").read_text(encoding="utf-8-sig")
            erf_descriptor = (tmp_path / "erf/Balances.xml").read_text(encoding="utf-8-sig")
            self.assertIn("<ExternalDataProcessor", epf_descriptor)
            self.assertIn("Import &amp; prices", epf_descriptor)
            self.assertIn("<Form>MainForm</Form>", epf_descriptor)
            self.assertIn("<ExternalReport", erf_descriptor)
            self.assertIn("<MainDataCompositionSchema/>", erf_descriptor)
            self.assertTrue((tmp_path / "epf/Import/Ext/ObjectModule.bsl").is_file())
            self.assertTrue((tmp_path / "epf/Import/Forms/MainForm/Ext/Form.xml").is_file())
            self.assertTrue((tmp_path / "erf/Balances/Ext/ObjectModule.bsl").is_file())
            source_sets = {
                source_set["name"]: source_set
                for source_set in json.loads(payloads[3]["stdout"])["sourceSets"]
            }
            self.assertEqual(source_sets["external-processors"]["kind"], "external_processor")
            self.assertEqual(source_sets["external-processors"]["sourceFormat"], "platform_xml")
            self.assertEqual(source_sets["external-reports"]["kind"], "external_report")
            self.assertEqual(source_sets["external-reports"]["sourceFormat"], "platform_xml")
