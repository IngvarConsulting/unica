from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parents[2]
SMOKE_SCRIPT = REPO_ROOT / "scripts" / "ci" / "smoke-unica-mcp.py"


def load_module():
    spec = importlib.util.spec_from_file_location("smoke_unica_mcp", SMOKE_SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SmokeUnicaMcpTests(unittest.TestCase):
    def run_smoke(self, server_source: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / "server.py"
            server.write_text(textwrap.dedent(server_source), encoding="utf-8")
            return subprocess.run(
                [
                    sys.executable,
                    str(SMOKE_SCRIPT),
                    "--binary",
                    sys.executable,
                    "--binary-arg",
                    str(server),
                    "--plugin-root",
                    str(root),
                    "--timeout-seconds",
                    "2",
                ],
                capture_output=True,
                text=True,
                check=False,
            )

    def test_accepts_initialize_and_required_tool_responses(self) -> None:
        result = self.run_smoke(
            """
            import json
            import sys

            for line in sys.stdin:
                message = json.loads(line)
                if message.get("method") == "initialize":
                    print(json.dumps({"jsonrpc": "2.0", "id": 1, "result": {}}), flush=True)
                elif message.get("method") == "tools/list":
                    tools = [
                        {"name": "unica.project.status"},
                        {"name": "unica.standards.search"},
                        {"name": "unica.standards.explain"},
                        {"name": "unica.dcs.compile"},
                        {"name": "unica.dcs.edit"},
                        {"name": "unica.dcs.info"},
                        {"name": "unica.dcs.validate"},
                    ]
                    print(json.dumps({"jsonrpc": "2.0", "id": 2, "result": {"tools": tools}}), flush=True)
            """
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("verified Unica MCP initialize and tools/list", result.stdout)

    def test_rejects_runtime_missing_a_required_tool(self) -> None:
        result = self.run_smoke(
            """
            import json
            import sys

            for line in sys.stdin:
                message = json.loads(line)
                if message.get("method") == "initialize":
                    print(json.dumps({"jsonrpc": "2.0", "id": 1, "result": {}}), flush=True)
                elif message.get("method") == "tools/list":
                    print(json.dumps({"jsonrpc": "2.0", "id": 2, "result": {"tools": []}}), flush=True)
            """
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unica.standards.explain", result.stderr)

    def test_rejects_runtime_exposing_a_removed_dcs_alias(self) -> None:
        result = self.run_smoke(
            """
            import json
            import sys

            for line in sys.stdin:
                message = json.loads(line)
                if message.get("method") == "initialize":
                    print(json.dumps({"jsonrpc": "2.0", "id": 1, "result": {}}), flush=True)
                elif message.get("method") == "tools/list":
                    tools = [
                        {"name": "unica.project.status"},
                        {"name": "unica.standards.search"},
                        {"name": "unica.standards.explain"},
                        {"name": "unica.dcs.compile"},
                        {"name": "unica.dcs.edit"},
                        {"name": "unica.dcs.info"},
                        {"name": "unica.dcs.validate"},
                        {"name": "unica.s" + "kd.compile"},
                    ]
                    print(json.dumps({"jsonrpc": "2.0", "id": 2, "result": {"tools": tools}}), flush=True)
            """
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("removed DCS aliases", result.stderr)

    def test_decodes_mcp_json_as_utf8_independently_of_windows_locale(self) -> None:
        module = load_module()
        tools = sorted(module.REQUIRED_TOOLS)
        responses = [
            {"jsonrpc": "2.0", "id": 1, "result": {"serverInfo": {"name": "Уника"}}},
            {
                "jsonrpc": "2.0",
                "id": 2,
                "result": {"tools": [{"name": name} for name in tools]},
            },
        ]
        stdout = "".join(json.dumps(value, ensure_ascii=False) + "\n" for value in responses)

        with mock.patch.object(
            module.subprocess,
            "run",
            return_value=subprocess.CompletedProcess(["unica"], 0, stdout, ""),
        ) as run:
            module.smoke(["unica"], Path("."), 20)

        self.assertEqual(run.call_args.kwargs["encoding"], "utf-8")


if __name__ == "__main__":
    unittest.main()
