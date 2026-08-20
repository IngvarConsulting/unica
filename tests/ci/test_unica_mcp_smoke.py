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

MUTATION_TOOL_NAMES = frozenset(
    {
        "unica.cf.edit",
        "unica.cf.init",
        "unica.support.edit",
        "unica.cfe.borrow",
        "unica.cfe.init",
        "unica.epf.init",
        "unica.erf.init",
        "unica.cfe.patch_method",
        "unica.meta.add",
        "unica.meta.edit",
        "unica.meta.remove",
        "unica.form.add",
        "unica.form.compile",
        "unica.form.edit",
        "unica.form.remove",
        "unica.interface.edit",
        "unica.subsystem.compile",
        "unica.subsystem.edit",
        "unica.dcs.compile",
        "unica.dcs.edit",
        "unica.mxl.compile",
        "unica.role.compile",
        "unica.role.edit",
        "unica.build.dump",
        "unica.build.load",
        "unica.build.update",
        "unica.build.make",
        "unica.build.run",
        "unica.runtime.execute",
        "unica.runtime.job.start",
        "unica.runtime.job.cancel",
        "unica.code.patch",
        "unica.xdto.edit",
    }
)


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
    def mcp_session(
        self,
        *,
        cache_dir: Path | None = None,
        workdir: Path | None = None,
    ):
        env = os.environ.copy()
        if cache_dir is not None:
            env["UNICA_CACHE_DIR"] = str(cache_dir)
        process = subprocess.Popen(
            [
                "cargo",
                "run",
                "--quiet",
                "--manifest-path",
                str(self.repo_root() / "Cargo.toml"),
                "--bin",
                "unica",
                "--",
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=workdir or self.repo_root(),
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
            try:
                if not process.stdin.closed:
                    try:
                        process.stdin.close()
                    except BrokenPipeError:
                        # The child may already be gone; reaping still has to run.
                        pass
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

    def test_tools_list_publishes_invocation_switch_only_for_mutations(self) -> None:
        responses = self.call_mcp(
            [{"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}]
        )
        tools = {
            tool["name"]: tool for tool in responses[0]["result"]["tools"]
        }

        self.assertEqual(len(tools), 71)
        self.assertEqual(len(MUTATION_TOOL_NAMES), 33)
        self.assertEqual(len(set(tools) - MUTATION_TOOL_NAMES), 38)
        self.assertEqual(MUTATION_TOOL_NAMES - set(tools), set())
        for name, tool in sorted(tools.items()):
            with self.subTest(tool=name):
                properties = tool["inputSchema"]["properties"]
                if confirm := properties.get("confirm"):
                    # Schema-only baseline (#479 §1): descriptions are absent;
                    # the hygiene rule applies once prose returns.
                    if confirm_description := confirm.get("description"):
                        self.assertNotIn(
                            "dryrun",
                            confirm_description.casefold(),
                        )
                if name in MUTATION_TOOL_NAMES:
                    dry_run = properties.get("dryRun")
                    self.assertIsNotNone(dry_run)
                    self.assertEqual(dry_run["type"], "boolean")
                    self.assertIs(dry_run.get("default"), True)
                else:
                    self.assertNotIn("dryRun", properties)

    def test_project_status_publishes_typed_readiness_without_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "workspace"
            (root / "src").mkdir(parents=True)
            (root / "src/Configuration.xml").write_text(
                "<MetaDataObject/>\n", encoding="utf-8"
            )
            (root / "v8project.yaml").write_text(
                "format: DESIGNER\n"
                "source-set:\n"
                "  - name: main\n"
                "    type: CONFIGURATION\n"
                "    path: src\n",
                encoding="utf-8",
            )
            before = snapshot_workspace_files(root)
            with self.mcp_session(
                cache_dir=Path(tmp) / "cache", workdir=root
            ) as request:
                response = request(
                    {
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.project.status",
                            "arguments": {"cwd": str(root)},
                        },
                    }
                )

            self.assertNotIn("error", response, response)
            self.assertFalse(response["result"]["isError"])
            payload = json.loads(response["result"]["content"][0]["text"])
            if "structuredContent" in response["result"]:
                self.assertEqual(payload, response["result"]["structuredContent"])
            self.assertTrue(payload["ok"])
            self.assertEqual(
                set(payload["data"]),
                {
                    "workspaceRoot",
                    "cacheRoot",
                    "ready",
                    "repositoryReady",
                    "checks",
                    "sourceSets",
                    "diagnostics",
                },
            )
            self.assertTrue(payload["data"]["ready"])
            self.assertFalse(payload["data"]["repositoryReady"])
            self.assertIn(
                "git.repository_absent",
                {item["code"] for item in payload["data"]["diagnostics"]},
            )
            self.assertEqual(snapshot_workspace_files(root), before)

    def test_reader_dry_run_rejection_precedes_workspace_and_target_resolution(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            missing = Path(tmp) / "missing-workspace"
            messages = []
            request_id = 1
            for name, base_arguments in [
                ("unica.project.status", {}),
                (
                    "unica.subsystem.info",
                    {"SubsystemPath": "missing/Subsystem.xml"},
                ),
            ]:
                for value in (True, False):
                    messages.append(
                        {
                            "jsonrpc": "2.0",
                            "id": request_id,
                            "method": "tools/call",
                            "params": {
                                "name": name,
                                "arguments": {
                                    **base_arguments,
                                    "cwd": str(missing),
                                    "dryRun": value,
                                },
                            },
                        }
                    )
                    request_id += 1

            responses = self.call_mcp(messages, cache_dir=Path(tmp) / "cache")

        self.assertEqual(len(responses), 4)
        for response in responses:
            with self.subTest(request_id=response["id"]):
                self.assertEqual(response["error"]["code"], -32000, response)
                self.assertIn(
                    "does not accept argument `dryRun`",
                    response["error"]["message"],
                )

    def test_representative_readers_execute_without_an_invocation_switch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            temp = Path(tmp)
            root = temp / "workspace"
            root.mkdir()
            root = root.resolve()
            with self.mcp_session(cache_dir=temp / "cache", workdir=root) as request:
                next_id = 2

                def call(name: str, arguments: dict) -> dict:
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
                    return response

                initialized = call(
                    "unica.cf.init",
                    {
                        "cwd": str(root),
                        "Name": "ReaderSmoke",
                        "OutputDir": "src",
                        "dryRun": False,
                    },
                )
                self.assertTrue(
                    json.loads(initialized["result"]["content"][0]["text"])["ok"]
                )
                (root / "v8project.yaml").write_text(
                    "format: DESIGNER\n"
                    "source-set:\n"
                    "  - name: main\n"
                    "    type: CONFIGURATION\n"
                    "    path: src\n",
                    encoding="utf-8",
                )
                added = call(
                    "unica.meta.add",
                    {
                        "sourceSet": "main",
                        "kind": "Catalog",
                        "name": "Smoke",
                        "dryRun": False,
                    },
                )
                self.assertTrue(added["result"]["structuredContent"]["ok"])
                subsystem = root / "standalone/Продажи.xml"
                subsystem.parent.mkdir(parents=True, exist_ok=True)
                (root / "standalone/Продажи/Ext").mkdir(parents=True)
                subsystem.write_text(
                    '<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" '
                    'version="2.20"><Subsystem><Properties><Name>Продажи</Name>'
                    '<IncludeInCommandInterface>true</IncludeInCommandInterface>'
                    '<Content xmlns:xr="http://v8.1c.ru/8.3/xcf/readable">'
                    '<xr:Item>Catalog.Smoke</xr:Item></Content></Properties>'
                    '</Subsystem></MetaDataObject>',
                    encoding="utf-8",
                )
                before = snapshot_workspace_files(root)

                calls = [
                    ("unica.project.status", {"cwd": str(root)}),
                    (
                        "unica.source.resolve",
                        {
                            "cwd": str(root),
                            "sourceSet": "main",
                            "query": "Catalog.Smoke",
                            "mode": "exact",
                        },
                    ),
                    (
                        "unica.meta.info",
                        {"sourceSet": "main", "metadataPath": "Catalog.Smoke"},
                    ),
                    (
                        "unica.subsystem.info",
                        {
                            "cwd": str(root),
                            "SubsystemPath": "standalone/Продажи.xml",
                        },
                    ),
                ]
                for name, arguments in calls:
                    with self.subTest(tool=name):
                        response = call(name, arguments)
                        payload = (
                            response["result"]["structuredContent"]
                            if name.startswith("unica.meta.")
                            else json.loads(response["result"]["content"][0]["text"])
                        )
                        self.assertTrue(payload["ok"], payload)
                        self.assertIn("data", payload)
                        summary = payload["summary"].lower()
                        self.assertNotIn("dry run", summary)
                        self.assertNotIn("preview", summary)

                self.assertEqual(snapshot_workspace_files(root), before)

    def test_meta_operations_stay_typed_without_conditional_evaluation(self) -> None:
        """The operation type must survive a host that renders only `properties`.

        `allOf`/`if`/`then` narrows `operations` per kind, but a client that does
        not evaluate conditionals or resolve `$ref` sees only
        `properties.operations`. Without a direct `items` there, the model is
        offered an untyped array and the whole typed contract is lost between the
        server and the caller, so this asserts the host-visible signature rather
        than the schema the validator eventually assembles.
        """
        responses = self.call_mcp(
            [
                {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": MCP_INITIALIZE_PARAMS},
                {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            ]
        )
        tools = {tool["name"]: tool for tool in responses[1]["result"]["tools"]}

        for name in ("unica.meta.add", "unica.meta.edit"):
            with self.subTest(tool=name):
                operations = tools[name]["inputSchema"]["properties"]["operations"]
                items = operations.get("items")
                self.assertIsNotNone(
                    items, f"{name}: operations publishes no direct items"
                )
                branches = items.get("oneOf")
                self.assertIsNotNone(
                    branches, f"{name}: operation items publish no discriminated union"
                )
                discriminators = set()
                for branch in branches:
                    self.assertNotIn(
                        "$ref", branch, f"{name}: a host that cannot resolve $ref sees nothing"
                    )
                    enum = branch["properties"]["op"]["enum"]
                    self.assertEqual(len(enum), 1)
                    discriminators.add(enum[0])
                    self.assertIn("op", branch["required"])
                self.assertEqual(
                    discriminators,
                    {"setProperties", "add", "update", "remove", "editRelations", "addHelp"},
                )

    def test_meta_calls_publish_structured_results(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "workspace"
            (root / "src").mkdir(parents=True)
            (root / "v8project.yaml").write_text(
                "format: DESIGNER\nsource-set:\n"
                "  - name: main\n    type: CONFIGURATION\n    path: src\n",
                encoding="utf-8",
            )
            fixture = (
                self.repo_root()
                / "tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware"
            )
            for source in fixture.rglob("*"):
                if source.is_file():
                    target = root / "src" / source.relative_to(fixture)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(source.read_bytes())
            with self.mcp_session(
                cache_dir=Path(tmp) / "cache", workdir=root
            ) as request:
                listed = request(
                    {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}
                )
                tools = {tool["name"]: tool for tool in listed["result"]["tools"]}
                schemas = [tools[name]["outputSchema"] for name in sorted(source_smoke_oracle().META_TOOL_NAMES)]
                self.assertTrue(all(schema == schemas[0] for schema in schemas[1:]))
                self.assertEqual(
                    schemas[0], source_smoke_oracle().EXPECTED_META_OUTPUT_SCHEMA
                )
                self.assertEqual(schemas[0]["type"], "object")
                self.assertFalse(schemas[0]["additionalProperties"])
                self.assertEqual(
                    schemas[0]["required"],
                    ["ok", "summary", "changes", "warnings", "errors", "artifacts", "cache"],
                )
                self.assertNotIn("outputSchema", tools["unica.project.status"])

                success = request(
                    {
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.meta.add",
                            "arguments": {
                                "sourceSet": "main",
                                "kind": "Catalog",
                                "name": "Items",
                            },
                        },
                    }
                )
                self.assertNotIn("error", success, success)
                success_result = success["result"]
                self.assertEqual(
                    json.loads(success_result["content"][0]["text"]),
                    success_result["structuredContent"],
                )
                self.assertTrue(success_result["structuredContent"]["ok"])
                self.assertFalse(success_result["isError"])

                invalid = request(
                    {
                        "jsonrpc": "2.0",
                        "id": 4,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.meta.info",
                            "arguments": {},
                        },
                    }
                )
                self.assertNotIn("error", invalid, invalid)
                invalid_result = invalid["result"]
                self.assertEqual(
                    json.loads(invalid_result["content"][0]["text"]),
                    invalid_result["structuredContent"],
                )
                self.assertFalse(invalid_result["structuredContent"]["ok"])
                self.assertEqual(
                    invalid_result["structuredContent"]["diagnostics"][0]["code"],
                    "invalid_arguments",
                )
                self.assertTrue(invalid_result["isError"])

    def test_role_edit_preview_publishes_only_typed_logical_data(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "workspace"
            rights = root / "src/Roles/Demo/Ext/Rights.xml"
            rights.parent.mkdir(parents=True)
            (root / "v8project.yaml").write_text(
                "format: DESIGNER\nsource-set:\n"
                "  - name: main\n    type: CONFIGURATION\n    path: src\n",
                encoding="utf-8",
            )
            (root / "src/Configuration.xml").write_text(
                '<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" '
                'version="2.20"><Configuration '
                'uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">'
                "<Properties><Name>Main</Name></Properties>"
                "<ChildObjects><Role>Demo</Role></ChildObjects>"
                "</Configuration></MetaDataObject>",
                encoding="utf-8",
            )
            (root / "src/Roles/Demo.xml").write_text(
                '<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" '
                'version="2.20"><Role '
                'uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">'
                "<Properties><Name>Demo</Name></Properties>"
                "</Role></MetaDataObject>",
                encoding="utf-8",
            )
            body = (
                '<?xml version="1.0" encoding="UTF-8"?>\r\n'
                '<Rights xmlns="http://v8.1c.ru/8.2/roles" '
                'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" '
                'xsi:type="Rights" version="2.20">\r\n'
                "\t<setForNewObjects>false</setForNewObjects>\r\n"
                "\t<setForAttributesByDefault>true</setForAttributesByDefault>\r\n"
                "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>\r\n"
                "\t<object>\r\n"
                "\t\t<name>Catalog.Demo</name>\r\n"
                "\t\t<right><name>Delete</name><value>true</value></right>\r\n"
                "\t</object>\r\n"
                "</Rights>\r\n"
            )
            before = b"\xef\xbb\xbf" + body.encode("utf-8")
            rights.write_bytes(before)

            with self.mcp_session(
                cache_dir=Path(tmp) / "cache", workdir=root
            ) as request:
                response = request(
                    {
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.role.edit",
                            "arguments": {
                                "sourceSet": "main",
                                "metadataPath": "Role.Demo",
                                "operations": [
                                    {
                                        "op": "setRight",
                                        "objectName": "Catalog.Demo",
                                        "right": "Delete",
                                        "value": False,
                                    }
                                ],
                                "dryRun": True,
                            },
                        },
                    }
                )

            self.assertNotIn("error", response, response)
            result = response["result"]
            payload = result["structuredContent"]
            self.assertEqual(json.loads(result["content"][0]["text"]), payload)
            self.assertTrue(payload["ok"])
            self.assertFalse(result["isError"])
            self.assertNotIn("stdout", payload)
            self.assertNotIn("stderr", payload)
            self.assertNotIn("command", payload)
            data = payload["data"]
            self.assertEqual(
                set(data),
                {"metadataPath", "changed", "effects", "validation", "diagnostics"},
            )
            self.assertEqual(data["metadataPath"], "Role.Demo")
            self.assertTrue(data["changed"])
            self.assertEqual(data["validation"], {"status": "passed"})
            self.assertEqual(data["diagnostics"], [])
            self.assertEqual(len(data["effects"]), 1)
            self.assertEqual(
                data["effects"][0],
                {
                    "operationIndex": 0,
                    "operation": "setRight",
                    "objectName": "Catalog.Demo",
                    "right": "Delete",
                    "before": True,
                    "after": False,
                    "action": "setRight",
                    "changed": True,
                },
            )
            encoded_data = json.dumps(data, ensure_ascii=False)
            self.assertNotIn("Rights.xml", encoded_data)
            self.assertNotIn(str(root), encoded_data)
            self.assertEqual(rights.read_bytes(), before)

    def test_role_edit_apply_publishes_through_the_transport(self) -> None:
        """Применение проходит тот же путь, что и предпросмотр.

        Предпросмотр ничего не публикует, поэтому сам по себе не доказывает,
        что apply через транспорт пишет файл, отдаёт тот же типизированный
        конверт и не выносит наружу физический путь.
        """

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "workspace"
            rights = root / "src/Roles/Demo/Ext/Rights.xml"
            rights.parent.mkdir(parents=True)
            (root / "v8project.yaml").write_text(
                "format: DESIGNER\nsource-set:\n"
                "  - name: main\n    type: CONFIGURATION\n    path: src\n",
                encoding="utf-8",
            )
            (root / "src/Configuration.xml").write_text(
                '<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" '
                'version="2.20"><Configuration '
                'uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">'
                "<Properties><Name>Main</Name></Properties>"
                "<ChildObjects><Role>Demo</Role></ChildObjects>"
                "</Configuration></MetaDataObject>",
                encoding="utf-8",
            )
            (root / "src/Roles/Demo.xml").write_text(
                '<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" '
                'version="2.20"><Role '
                'uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">'
                "<Properties><Name>Demo</Name></Properties>"
                "</Role></MetaDataObject>",
                encoding="utf-8",
            )
            body = (
                '<?xml version="1.0" encoding="UTF-8"?>\r\n'
                '<Rights xmlns="http://v8.1c.ru/8.2/roles" '
                'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" '
                'xsi:type="Rights" version="2.20">\r\n'
                "\t<setForNewObjects>false</setForNewObjects>\r\n"
                "\t<setForAttributesByDefault>true</setForAttributesByDefault>\r\n"
                "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>\r\n"
                "\t<object>\r\n"
                "\t\t<name>Catalog.Demo</name>\r\n"
                "\t\t<right><name>Delete</name><value>true</value></right>\r\n"
                "\t\t<right><name>Read</name><value>true</value></right>\r\n"
                "\t</object>\r\n"
                "</Rights>\r\n"
            )
            before = b"\xef\xbb\xbf" + body.encode("utf-8")
            rights.write_bytes(before)

            call = {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "unica.role.edit",
                    "arguments": {
                        "sourceSet": "main",
                        "metadataPath": "Role.Demo",
                        "operations": [
                            {
                                "op": "setRight",
                                "objectName": "Catalog.Demo",
                                "right": "Delete",
                                "value": False,
                            }
                        ],
                        "dryRun": False,
                    },
                },
            }
            with self.mcp_session(
                cache_dir=Path(tmp) / "cache", workdir=root
            ) as request:
                response = request(call)

            self.assertNotIn("error", response, response)
            result = response["result"]
            payload = result["structuredContent"]
            self.assertEqual(json.loads(result["content"][0]["text"]), payload)
            self.assertTrue(payload["ok"])
            self.assertFalse(result["isError"])
            for absent in ("stdout", "stderr", "command"):
                self.assertNotIn(absent, payload)
            data = payload["data"]
            self.assertEqual(
                set(data),
                {"metadataPath", "changed", "effects", "validation", "diagnostics"},
            )
            self.assertTrue(data["changed"])
            self.assertEqual(data["validation"], {"status": "passed"})
            self.assertEqual(data["effects"][0]["before"], True)
            self.assertEqual(data["effects"][0]["after"], False)
            self.assertEqual(payload["cache"]["events"], ["RoleChanged"])
            self.assertNotEqual(payload["cache"]["mode"], "dry-run")

            # Ни одна часть конверта применения не несёт физический путь.
            encoded = json.dumps(payload, ensure_ascii=False)
            self.assertNotIn("Rights.xml", encoded)
            self.assertNotIn(str(root), encoded)

            # Платформенная форма: право со значением умолчания не хранится.
            published = rights.read_bytes()
            self.assertNotEqual(published, before)
            self.assertTrue(published.startswith(b"\xef\xbb\xbf"))
            published_text = published.decode("utf-8-sig")
            self.assertNotIn("<name>Delete</name>", published_text)
            self.assertIn("<name>Read</name>", published_text)
            self.assertNotIn("\n\n", published_text.replace("\r\n", "\n"))

            # Повтор того же вызова — семантический no-op без записи.
            with self.mcp_session(
                cache_dir=Path(tmp) / "cache2", workdir=root
            ) as request:
                repeated = request(call)
            repeated_data = repeated["result"]["structuredContent"]["data"]
            self.assertFalse(repeated_data["changed"])
            self.assertEqual(rights.read_bytes(), published)

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
                oracle._stable_tool_contract(
                    listed["result"]["tools"],
                    oracle.expected_tool_names(
                        self.repo_root() / "plugins" / "unica"
                    ),
                )
                tools = {item["name"]: item for item in listed["result"]["tools"]}
                source_tools = {
                    "unica.source.resolve",
                    "unica.source.children",
                    "unica.source.resources",
                    "unica.source.read",
                    "unica.source.locate",
                }
                self.assertTrue(source_tools.issubset(tools))
                for name in source_tools:
                    schema = tools[name]["inputSchema"]
                    # `unica.source.locate` takes a path as the subject it
                    # translates, not as a target selector; every other source
                    # tool selects its target logically.
                    forbidden = {"sourceDir", "provider", "handle"}
                    if name != "unica.source.locate":
                        forbidden.add("path")
                    self.assertFalse(forbidden & set(schema["properties"]), name)
                    if name != "unica.source.locate":
                        assert_no_physical_source_selectors(schema)

                for source_set in ("main", "extension"):
                    target = "CommonModule.Shared.Module"
                    before_flow = snapshot_workspace_files(root)
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
                    # The whole source surface is read-only, so the session must
                    # leave every byte of the workspace in place.
                    self.assertEqual(
                        snapshot_workspace_files(root),
                        before_flow,
                        f"{source_set} read-only flow changed workspace bytes",
                    )
                    located = tool(
                        "unica.source.locate",
                        {"cwd": str(root), "sourceSet": source_set,
                         "path": f"{'src' if source_set == 'main' else 'ext'}/CommonModules/Shared/Ext/Module.bsl"},
                    )
                    self.assertEqual(located["data"]["metadataPath"], target)
                    self.assertEqual(
                        oracle.source_flow_projection(
                            source_set,
                            cache_root,
                            resolve,
                            children,
                            resources,
                            read,
                        ),
                        oracle.expected_source_flow_projection(source_set),
                    )

            # The public source surface is read-only end to end.
            self.assertEqual(snapshot_workspace_files(root), before)

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
                # Every snapshotted descriptor stays read-only: nothing in the
                # manifest may advertise a write the surface cannot perform.
                for resource in payload["data"]["resources"]:
                    self.assertEqual(resource["access"], ["read"], resource)
                partial_response = raw_tool(
                    "unica.source.resources",
                    {"cwd": str(root), "sourceSet": "main", "scope": "aggregate", "limit": 1},
                )
                self.assertNotIn("error", partial_response)
                partial_payload = json.loads(
                    partial_response["result"]["content"][0]["text"]
                )
                self.assertEqual(partial_payload["data"]["completeness"], "partial")
                # The writer is gone from the public surface entirely.
                removed = raw_tool("unica.source.apply", {"cwd": str(root)})
                assert_rpc_error(
                    removed,
                    prefix="unknown unica tool",
                    hint="unica.source.apply",
                )

    def test_one_logical_address_flows_from_resolve_through_meta_info(self) -> None:
        """An address found logically must be readable by the subject-matter
        tool without ever spelling out a configurator layout path."""

        with tempfile.TemporaryDirectory() as tmp:
            temp = Path(tmp)
            root = temp / "workspace"
            self.source_fixture(root)
            configuration = root / "src/Configuration.xml"
            configuration.write_bytes(
                configuration.read_bytes().replace(
                    b"</ChildObjects>",
                    b"<Catalog>Goods</Catalog></ChildObjects>",
                )
            )
            (root / "src/Catalogs").mkdir(parents=True)
            (root / "src/Catalogs/Goods.xml").write_bytes(
                (
                    "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">"
                    "<Catalog uuid=\"7e082488-1a41-4d06-b4be-66265c55ca16\">"
                    "<Properties><Name>Goods</Name>"
                    "<Owners><xr:Item xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" "
                    "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" "
                    "xsi:type=\"xr:MDObjectRef\">"
                    "Catalog.Kinds</xr:Item></Owners>"
                    "</Properties></Catalog></MetaDataObject>"
                ).encode()
            )
            before = snapshot_workspace_files(root)
            with self.mcp_session(cache_dir=temp / "cache", workdir=root) as request:
                next_id = 2

                def call(name: str, arguments: dict) -> dict:
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

                resolved = json.loads(
                    call(
                        "unica.source.resolve",
                        {
                            "cwd": str(root),
                            "sourceSet": "main",
                            "query": "Справочник.Goods",
                            "mode": "exact",
                        },
                    )["result"]["content"][0]["text"]
                )
                address = resolved["data"]["candidates"][0]["metadataPath"]
                self.assertEqual(address, "Catalog.Goods")

                manifest = json.loads(
                    call(
                        "unica.source.resources",
                        {
                            "cwd": str(root),
                            "sourceSet": "main",
                            "metadataPath": address,
                            "scope": "self",
                        },
                    )["result"]["content"][0]["text"]
                )
                self.assertEqual(manifest["data"]["completeness"], "complete")
                self.assertEqual(
                    [item["role"] for item in manifest["data"]["resources"]],
                    ["metadataDescriptor"],
                )
                assert_no_physical_source_selectors(manifest["data"])

                info = json.loads(
                    call(
                        "unica.meta.info",
                        {
                            "sourceSet": "main",
                            "metadataPath": address,
                        },
                    )["result"]["content"][0]["text"]
                )
                # This hand-written descriptor is intentionally incomplete.
                # The typed reader still returns its local structure while
                # making the semantic validation failure explicit.
                self.assertFalse(info["ok"], info)
                self.assertEqual(info["data"]["metadataPath"], address)
                # The owners used to be a printed line; ADR-0023 makes them data.
                self.assertEqual(
                    info["data"]["relations"]["owners"],
                    [{"kind": "object", "value": "Catalog.Kinds"}],
                )

                legacy = call(
                    "unica.meta.info",
                    {"ObjectPath": "src/Catalogs/Goods.xml"},
                )
                self.assertNotIn("error", legacy, legacy)
                legacy_result = legacy["result"]
                legacy_payload = json.loads(legacy_result["content"][0]["text"])
                self.assertEqual(legacy_result["structuredContent"], legacy_payload)
                self.assertTrue(legacy_result["isError"])
                self.assertFalse(legacy_payload["ok"])
                self.assertEqual(
                    legacy_payload["diagnostics"][0]["code"], "invalid_arguments"
                )
                self.assertIn(
                    "does not accept argument `ObjectPath`",
                    legacy_payload["diagnostics"][0]["message"],
                )
                self.assertIn("sourceSet", legacy_payload["diagnostics"][0]["message"])
                self.assertIn("metadataPath", legacy_payload["diagnostics"][0]["message"])

                missing = call(
                    "unica.source.resources",
                    {
                        "cwd": str(root),
                        "sourceSet": "main",
                        "metadataPath": "Catalog.Missing",
                        "scope": "self",
                    },
                )
                # An address nobody can prove is a refusal, not an outage: the
                # caller must not read it as something to retry.
                self.assertNotIn("result", missing)
                self.assertTrue(
                    missing["error"]["message"].startswith("target_not_found:"),
                    missing,
                )
                self.assertNotIn("source_unavailable", missing["error"]["message"])

            self.assertEqual(snapshot_workspace_files(root), before)

    def test_four_typed_meta_tools_run_through_jsonrpc_on_a_real_fixture(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            temp = Path(tmp)
            root = temp / "workspace"
            root.mkdir()

            with self.mcp_session(cache_dir=temp / "cache", workdir=root) as request:
                next_id = 2

                def raw_call(name: str, arguments: dict) -> dict:
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

                def call(name: str, arguments: dict) -> dict:
                    response = raw_call(name, arguments)
                    self.assertNotIn("error", response, response)
                    payload = json.loads(response["result"]["content"][0]["text"])
                    self.assertTrue(payload["ok"], payload)
                    self.assertIn("data", payload)
                    return payload

                initialized = call(
                    "unica.cf.init",
                    {
                        "cwd": str(root),
                        "Name": "TypedMetaSmoke",
                        "OutputDir": "src",
                        "dryRun": False,
                    },
                )
                self.assertTrue(initialized["changes"])
                (root / "v8project.yaml").write_text(
                    "format: DESIGNER\n"
                    "source-set:\n"
                    "  - name: main\n"
                    "    type: CONFIGURATION\n"
                    "    path: src\n",
                    encoding="utf-8",
                )

                added = call(
                    "unica.meta.add",
                    {
                        "sourceSet": "main",
                        "kind": "Catalog",
                        "name": "Smoke",
                        "dryRun": False,
                    },
                )
                self.assertEqual(added["data"]["metadataPath"], "Catalog.Smoke")
                self.assertTrue(added["data"]["changed"])
                self.assertTrue(added["cache"]["events"])

                info = call(
                    "unica.meta.info",
                    {"sourceSet": "main", "metadataPath": "Catalog.Smoke"},
                )
                self.assertEqual(info["data"]["metadataPath"], "Catalog.Smoke")
                self.assertEqual(info["data"]["validation"]["status"], "passed")

                edited = call(
                    "unica.meta.edit",
                    {
                        "sourceSet": "main",
                        "metadataPath": "Catalog.Smoke",
                        "operations": [
                            {"op": "setProperties", "values": {"Comment": "JSON-RPC"}}
                        ],
                        "dryRun": False,
                    },
                )
                self.assertTrue(edited["data"]["changed"])
                self.assertTrue(edited["cache"]["events"])

                removed = call(
                    "unica.meta.remove",
                    {
                        "sourceSet": "main",
                        "metadataPath": "Catalog.Smoke",
                        "dryRun": False,
                    },
                )
                self.assertTrue(removed["data"]["changed"])
                self.assertTrue(removed["cache"]["events"])

                for retired in [
                    "unica.meta.compile",
                    "unica.meta.profile",
                    "unica.meta.validate",
                ]:
                    response = raw_call(retired, {})
                    self.assertEqual(response["error"]["code"], -32000, response)
                    self.assertIn("unknown unica tool", response["error"]["message"])
                    self.assertIn(retired, response["error"]["message"])

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

    def test_runtime_execute_dry_run_reports_a_missing_runner_honestly(self) -> None:
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
        self.assertFalse(payload["ok"])
        self.assertEqual(payload["cache"]["mode"], "read")
        self.assertTrue(
            any("bundled_tool_missing" in error for error in payload["errors"]),
            payload,
        )
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
            # ADR-0023: the map is typed data, not a JSON string in stdout.
            self.assertIsNone(payloads[3].get("stdout"))
            source_sets = {
                source_set["name"]: source_set
                for source_set in payloads[3]["data"]["sourceSets"]
            }
            self.assertEqual(source_sets["external-processors"]["kind"], "external_processor")
            self.assertEqual(source_sets["external-processors"]["sourceFormat"], "platform_xml")
            self.assertEqual(source_sets["external-reports"]["kind"], "external_report")
            self.assertEqual(source_sets["external-reports"]["sourceFormat"], "platform_xml")
