from __future__ import annotations

import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SMOKE_SCRIPT = REPO_ROOT / "scripts" / "ci" / "smoke-unica-mcp.py"


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


if __name__ == "__main__":
    unittest.main()
