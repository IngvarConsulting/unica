from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SMOKE_SCRIPT = REPO_ROOT / "scripts" / "ci" / "smoke-unica-mcp.py"


class SmokeUnicaMcpTests(unittest.TestCase):
    def run_smoke(
        self,
        server_source: str,
        *,
        environment: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / "server.py"
            server.write_text(textwrap.dedent(server_source), encoding="utf-8")
            process_environment = os.environ.copy()
            process_environment.update(environment or {})
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
                encoding="utf-8",
                errors="strict",
                check=False,
                env=process_environment,
                timeout=8,
            )

    def valid_discovery(self) -> dict:
        return {
            "status": "partial",
            "candidates": [
                {
                    "target": (
                        "Document.ПриобретениеТоваровУслуг."
                        "TabularSection.Серии"
                    ),
                    "recommendation": {
                        "summary": "Review typed metadata evidence.",
                        "basis": ["metadata_structure"],
                    },
                },
                {
                    "target": "DataProcessor.ПодборСерийВДокументы",
                    "recommendation": {
                        "summary": "Review typed metadata evidence.",
                        "basis": ["metadata_structure"],
                    },
                },
                {
                    "target": (
                        "DataProcessor.ПодборСерийВДокументы.Form."
                        "РегистрацияИПодборСерийПоОднойСтрокеТоваров"
                    ),
                    "recommendation": {
                        "summary": "Review typed form-binding evidence.",
                        "basis": ["managed_form_binding"],
                    },
                },
            ],
            "warnings": [
                {
                    "code": "alternative_relevant_tabular_section",
                    "blocking": True,
                }
            ],
            "missingChecks": [{"code": "bsl_index_missing"}],
        }

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

            def emit(value):
                encoded = (json.dumps(value, ensure_ascii=False) + "\\n").encode("utf-8")
                sys.stdout.buffer.write(encoded)
                sys.stdout.buffer.flush()

            for raw in sys.stdin.buffer:
                message = json.loads(raw.decode("utf-8", errors="strict"))
                if message.get("method") == "initialize":
                    emit({{"jsonrpc": "2.0", "id": 1, "result": {{}}}})
                elif message.get("method") == "tools/list":
                    emit({{
                        "jsonrpc": "2.0",
                        "id": 2,
                        "result": {{"tools": [{{"name": name}} for name in tools]}},
                    }})
                elif message.get("method") == "tools/call":
                    params = message["params"]
                    assert params["name"] == "unica.project.discover"
                    assert set(params["arguments"]) == {{"mode", "task"}}
                    assert params["arguments"]["mode"] == "explore"
                    assert params["arguments"]["task"]
                    emit({{
                        "jsonrpc": "2.0",
                        "id": message["id"],
                        "result": {{"content": [{{"type": "text", "text": payload_text}}]}},
                    }})
        """

    def test_accepts_initialize_tools_and_task_only_discovery(self) -> None:
        result = self.run_smoke(
            self.discovery_server(
                {"ok": True, "data": {"discovery": self.valid_discovery()}}
            )
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("verified Unica MCP task-only discovery", result.stdout)

    def test_accepts_cyrillic_protocol_under_ascii_locale(self) -> None:
        result = self.run_smoke(
            self.discovery_server(
                {"ok": True, "data": {"discovery": self.valid_discovery()}}
            ),
            environment={
                "PYTHONUTF8": "0",
                "PYTHONCOERCECLOCALE": "0",
                "LC_ALL": "C",
                "LANG": "C",
            },
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
        result = self.run_smoke(self.discovery_server({"ok": True, "data": {}}))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("data.discovery is missing", result.stderr)

    def test_rejects_malformed_discovery_data(self) -> None:
        result = self.run_smoke(
            self.discovery_server({"ok": True, "data": {"discovery": []}})
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("data.discovery must be an object", result.stderr)

    def test_rejects_operation_result_without_success(self) -> None:
        discovery = self.valid_discovery()
        discovery["candidates"].append(
            {"target": "Document.ПриобретениеТоваровУслуг"}
        )
        result = self.run_smoke(
            self.discovery_server(
                {"ok": False, "data": {"discovery": discovery}}
            )
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("OperationResult.ok must be true", result.stderr)

    def test_rejects_root_document_instead_of_series_tabular_section(self) -> None:
        discovery = self.valid_discovery()
        discovery["candidates"][0]["target"] = "Document.ПриобретениеТоваровУслуг"
        result = self.run_smoke(
            self.discovery_server({"ok": True, "data": {"discovery": discovery}})
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "Document.ПриобретениеТоваровУслуг.TabularSection.Серии",
            result.stderr,
        )

    def test_rejects_discovery_without_alternative_relevant_section_warning(self) -> None:
        discovery = self.valid_discovery()
        discovery["warnings"] = [{"code": "unrelated_warning", "blocking": True}]
        result = self.run_smoke(
            self.discovery_server({"ok": True, "data": {"discovery": discovery}})
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("alternative_relevant_tabular_section", result.stderr)

    def test_rejects_candidate_without_typed_recommendation(self) -> None:
        discovery = self.valid_discovery()
        discovery["candidates"][0].pop("recommendation")
        result = self.run_smoke(
            self.discovery_server({"ok": True, "data": {"discovery": discovery}})
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("candidate recommendation is missing", result.stderr)

    def test_rejects_non_string_recommendation_basis_without_traceback(self) -> None:
        discovery = self.valid_discovery()
        discovery["candidates"][0]["recommendation"]["basis"] = [
            {"unexpected": "object"}
        ]
        result = self.run_smoke(
            self.discovery_server({"ok": True, "data": {"discovery": discovery}})
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("candidate recommendation basis is invalid", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_preserves_protocol_error_when_stdin_close_is_broken(self) -> None:
        started = time.monotonic()
        result = self.run_smoke(
            """
            import json
            import os
            import sys
            import time

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
            for line in sys.stdin:
                message = json.loads(line)
                if message.get("method") == "initialize":
                    print(json.dumps({"jsonrpc": "2.0", "id": 1, "result": {}}), flush=True)
                elif message.get("method") == "tools/list":
                    os.close(0)
                    print(json.dumps({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "result": {"tools": [{"name": name} for name in tools]},
                    }), flush=True)
                    print("fake MCP closed fd 0", file=sys.stderr, flush=True)
                    time.sleep(4)
                    raise SystemExit(0)
            """
        )
        elapsed = time.monotonic() - started

        self.assertNotEqual(result.returncode, 0)
        self.assertLess(elapsed, 3)
        self.assertIn("closed stdin before responding", result.stderr)
        self.assertIn("fake MCP closed fd 0", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

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
