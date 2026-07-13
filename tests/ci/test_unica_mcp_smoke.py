from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


class UnicaMcpSmokeTests(unittest.TestCase):
    def repo_root(self) -> Path:
        return Path(__file__).resolve().parents[2]

    def call_mcp(self, messages: list[dict], *, cache_dir: Path | None = None) -> list[dict]:
        env = os.environ.copy()
        if cache_dir is not None:
            env["UNICA_CACHE_DIR"] = str(cache_dir)
        payload = "\n".join(json.dumps(message) for message in messages) + "\n"
        result = subprocess.run(
            ["cargo", "run", "--quiet", "--bin", "unica", "--"],
            input=payload,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=True,
            cwd=self.repo_root(),
            env=env,
        )
        return [json.loads(line) for line in result.stdout.splitlines() if line.strip()]

    def test_initialize_lists_single_unica_server(self) -> None:
        responses = self.call_mcp(
            [
                {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
                {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            ]
        )

        self.assertEqual(responses[0]["result"]["serverInfo"]["name"], "unica")
        tools = {tool["name"] for tool in responses[1]["result"]["tools"]}
        self.assertIn("unica.project.status", tools)
        self.assertIn("unica.project.map", tools)
        self.assertIn("unica.project.discover", tools)
        self.assertIn("unica.form.edit", tools)
        self.assertIn("unica.build.load", tools)
        self.assertIn("unica.runtime.execute", tools)
        self.assertIn("unica.standards.explain", tools)

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
        command = " ".join(payload["command"])
        self.assertIn("bin/", command)
        self.assertIn("v8-runner", command)
        self.assertNotIn("run-v8-runner.sh", command)

    def test_project_discover_returns_task_only_fixture_flow_as_structured_data(self) -> None:
        fixture = self.repo_root() / "tests/fixtures/extension-point-discovery/ut115"
        with tempfile.TemporaryDirectory() as tmp:
            responses = self.call_mcp(
                [
                    {
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.project.discover",
                            "arguments": {
                                "cwd": str(fixture),
                                "task": "При поступлении проверять остаточный срок годности серий",
                            },
                        },
                    }
                ],
                cache_dir=Path(tmp) / "cache",
            )

        payload = json.loads(responses[0]["result"]["content"][0]["text"])
        self.assertTrue(payload["ok"])
        self.assertNotIn("stdout", payload)
        self.assertNotIn("bsl_diagnostics", payload["cache"]["lazy_rebuilt"])
        self.assertNotIn("bsl_diagnostics", payload["cache"]["fresh"])
        data = payload["data"]
        self.assertEqual(data["schemaVersion"], 1)
        self.assertEqual(data["status"], "partial")
        self.assertNotIn("task", data)
        objects = {candidate["object"] for candidate in data["candidateExtensionPoints"]}
        self.assertIn(
            "Document.ПриобретениеТоваровУслуг.TabularSection.Серии",
            objects,
        )
        self.assertIn("DataProcessor.ПодборСерийВДокументы", objects)
        self.assertIn(
            "DataProcessor.ПодборСерийВДокументы.Form.РегистрацияИПодборСерийПоОднойСтрокеТоваров",
            objects,
        )
        lexical = [
            evidence
            for evidence in data["evidence"]
            if evidence["source"] == "bsl_lexical_scan"
        ]
        self.assertTrue(lexical)
        self.assertTrue(all(evidence.get("line", 0) > 0 for evidence in lexical))
        self.assertTrue(
            all(
                not Path(evidence["path"]).is_absolute()
                for evidence in data["evidence"]
                if "path" in evidence
            )
        )
