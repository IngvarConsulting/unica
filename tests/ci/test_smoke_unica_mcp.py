from __future__ import annotations

import json
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

    def discovery_server(self, operation_payload: dict | list) -> str:
        payload_text = json.dumps(operation_payload, ensure_ascii=False)
        return f"""
            import json
            import sys

            tools = [
                "unica.project.status",
                "unica.project.discover",
                "unica.standards.search",
                "unica.standards.explain",
                "unica.dcs.compile",
                "unica.dcs.edit",
                "unica.dcs.info",
                "unica.dcs.validate",
            ]
            payload_text = {payload_text!r}
            for line in sys.stdin:
                message = json.loads(line)
                if message.get("method") == "initialize":
                    print(json.dumps({{"jsonrpc": "2.0", "id": 1, "result": {{}}}}), flush=True)
                elif message.get("method") == "tools/list":
                    print(json.dumps({{
                        "jsonrpc": "2.0",
                        "id": 2,
                        "result": {{"tools": [{{"name": name}} for name in tools]}},
                    }}), flush=True)
                elif message.get("method") == "tools/call":
                    params = message["params"]
                    assert params["name"] == "unica.project.discover"
                    assert set(params["arguments"]) == {{"mode", "task"}}
                    assert params["arguments"]["mode"] == "explore"
                    assert params["arguments"]["task"]
                    print(json.dumps({{
                        "jsonrpc": "2.0",
                        "id": message["id"],
                        "result": {{"content": [{{"type": "text", "text": payload_text}}]}},
                    }}, ensure_ascii=False), flush=True)
        """

    def test_accepts_initialize_tools_and_task_only_discovery(self) -> None:
        discovery = {
            "status": "partial",
            "candidates": [
                {"target": "document.приобретениетоваровуслуг"},
                {"target": "dataprocessor.подборсерийвдокументы"},
                {
                    "target": (
                        "document.приобретениетоваровуслуг.form."
                        "регистрацияиподборсерийпооднойстрокетоваров"
                    )
                },
            ],
            "warnings": [
                {"code": "separate_series_section", "blocking": True}
            ],
            "missingChecks": [{"code": "bsl_index_missing"}],
        }

        result = self.run_smoke(
            self.discovery_server({"data": {"discovery": discovery}})
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("verified Unica MCP task-only discovery", result.stdout)

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

    def test_preserves_early_process_stderr(self) -> None:
        result = self.run_smoke(
            """
            import sys

            sys.stderr.write("fatal packaged runtime\\n")
            sys.stderr.flush()
            raise SystemExit(7)
            """
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("fatal packaged runtime", result.stderr)

    def test_rejects_runtime_missing_discovery_tool(self) -> None:
        result = self.run_smoke(
            """
            import json
            import sys

            tools = [
                "unica.project.status",
                "unica.standards.search",
                "unica.standards.explain",
                "unica.dcs.compile",
                "unica.dcs.edit",
                "unica.dcs.info",
                "unica.dcs.validate",
            ]
            for line in sys.stdin:
                message = json.loads(line)
                if message.get("method") == "initialize":
                    print(json.dumps({"jsonrpc": "2.0", "id": 1, "result": {}}), flush=True)
                elif message.get("method") == "tools/list":
                    print(json.dumps({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "result": {"tools": [{"name": name} for name in tools]},
                    }), flush=True)
            """
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unica.project.discover", result.stderr)

    def test_rejects_missing_discovery_data(self) -> None:
        result = self.run_smoke(self.discovery_server({"data": {}}))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("data.discovery is missing", result.stderr)

    def test_rejects_malformed_discovery_data(self) -> None:
        result = self.run_smoke(self.discovery_server({"data": {"discovery": []}}))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("data.discovery must be an object", result.stderr)

    def test_rejects_discovery_without_separate_series_warning(self) -> None:
        discovery = {
            "status": "partial",
            "candidates": [
                {"target": "document.приобретениетоваровуслуг"},
                {"target": "dataprocessor.подборсерийвдокументы"},
                {
                    "target": (
                        "document.приобретениетоваровуслуг.form."
                        "регистрацияиподборсерийпооднойстрокетоваров"
                    )
                },
            ],
            "warnings": [{"code": "unrelated_warning", "blocking": True}],
            "missingChecks": [{"code": "bsl_index_missing"}],
        }
        result = self.run_smoke(
            self.discovery_server({"data": {"discovery": discovery}})
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("separate_series_section", result.stderr)

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
                        {"name": "unica.project.discover"},
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
