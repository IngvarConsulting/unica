from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import textwrap
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
            review_path = root / "spec/architecture/tool-surface-review.json"
            review_path.parent.mkdir(parents=True)
            review_path.write_text(
                json.dumps({"unica.xdto.info": {}, "unica.xdto.edit": {}}),
                encoding="utf-8",
            )
            source_plugin = root / "plugins/unica"
            packaged_plugin = root / ".build/thin/marketplace/plugins/unica"
            source_plugin.mkdir(parents=True)
            packaged_plugin.mkdir(parents=True)

            for plugin_root in (source_plugin, packaged_plugin):
                with self.subTest(plugin_root=plugin_root):
                    self.assertEqual(
                        module.expected_tool_names(plugin_root),
                        {"unica.xdto.info", "unica.xdto.edit"},
                    )

    def run_smoke(
        self,
        tools: set[str],
        *,
        server_name: str = "unica",
        schema_drift: bool = False,
        result_drift: bool = False,
        provider_revision: bool = False,
        read_writes: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        module = load_module()
        expected_tools = self.expected_tools()
        source_schemas = json.loads(
            json.dumps(module.EXPECTED_SOURCE_INPUT_SCHEMAS)
        )
        source_flows = json.loads(
            json.dumps(module.EXPECTED_SOURCE_FLOW_PROJECTIONS)
        )
        if schema_drift:
            source_schemas["unica.source.read"]["required"].remove("resourceId")
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

            tools = __TOOLS__
            source_schemas = json.loads(r'''__SOURCE_SCHEMAS__''')
            source_flows = json.loads(r'''__SOURCE_FLOWS__''')
            read_writes = __READ_WRITES__

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

            for line in sys.stdin:
                message = json.loads(line)
                request_id = message.get("id")
                if message.get("method") == "initialize":
                    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": {"serverInfo": {"name": __NAME__}}}), flush=True)
                elif message.get("method") == "tools/list":
                    listed = []
                    for name in tools:
                        schema = source_schemas.get(
                            name,
                            {
                                "type": "object",
                                "properties": {},
                                "required": [],
                                "additionalProperties": False,
                            },
                        )
                        listed.append({"name": name, "inputSchema": schema})
                    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": {"tools": listed}}), flush=True)
                elif message.get("method") == "tools/call":
                    name = message["params"]["name"]
                    args = message["params"]["arguments"]
                    payload = source_payload(name, args)
                    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": {"content": [{"type": "text", "text": json.dumps(payload)}]}}), flush=True)
        """)
        server_source = (
            server_source.replace("__TOOLS__", json.dumps(sorted(tools)))
            .replace(
                "__SOURCE_SCHEMAS__",
                json.dumps(source_schemas, ensure_ascii=False),
            )
            .replace(
                "__SOURCE_FLOWS__",
                json.dumps(source_flows, ensure_ascii=False),
            )
            .replace("__READ_WRITES__", repr(read_writes))
            .replace("__NAME__", repr(server_name))
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
        result = self.run_smoke(self.expected_tools())

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("verified packaged Unica MCP source-resource flow", result.stdout)

    def test_rejects_runtime_missing_a_required_tool(self) -> None:
        result = self.run_smoke(self.expected_tools() - {"unica.xdto.edit"})

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing", result.stderr)
        self.assertIn("unica.xdto.edit", result.stderr)

    def test_rejects_runtime_exposing_an_unexpected_tool(self) -> None:
        result = self.run_smoke(self.expected_tools() | {"unica.xdto.validate"})

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unexpected", result.stderr)
        self.assertIn("unica.xdto.validate", result.stderr)

    def test_reports_missing_and_unexpected_tools_together(self) -> None:
        tools = self.expected_tools() - {"unica.xdto.edit"}
        tools.add("unica.xdto.validate")
        result = self.run_smoke(tools)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing", result.stderr)
        self.assertIn("unica.xdto.edit", result.stderr)
        self.assertIn("unexpected", result.stderr)
        self.assertIn("unica.xdto.validate", result.stderr)

    def test_decodes_mcp_json_as_utf8_independently_of_windows_locale(self) -> None:
        result = self.run_smoke(self.expected_tools(), server_name="Уника")

        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_incomplete_source_schema(self) -> None:
        result = self.run_smoke(self.expected_tools(), schema_drift=True)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("schema", result.stderr)

    def test_rejects_stable_source_result_drift(self) -> None:
        result = self.run_smoke(self.expected_tools(), result_drift=True)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stable", result.stderr)

    def test_rejects_provider_revision_leakage(self) -> None:
        result = self.run_smoke(
            self.expected_tools(),
            provider_revision=True,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("providerRevision", result.stderr)

    def test_rejects_a_read_that_writes(self) -> None:
        """The whole source surface is read-only, so any byte it changes fails."""
        result = self.run_smoke(self.expected_tools(), read_writes=True)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("read-only", result.stderr)


if __name__ == "__main__":
    unittest.main()
