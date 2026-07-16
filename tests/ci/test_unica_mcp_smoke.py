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
        self.assertIn("unica.form.edit", tools)
        self.assertIn("unica.epf.init", tools)
        self.assertIn("unica.erf.init", tools)
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

            payloads = [
                json.loads(response["result"]["content"][0]["text"])
                for response in responses
            ]
            self.assertTrue(payloads[0]["ok"], payloads[0])
            self.assertTrue(payloads[1]["ok"], payloads[1])
            self.assertEqual(len(payloads[0]["artifacts"]), 5)
            self.assertEqual(len(payloads[1]["artifacts"]), 2)

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
                for source_set in json.loads(payloads[2]["stdout"])["sourceSets"]
            }
            self.assertEqual(source_sets["external-processors"]["kind"], "external_processor")
            self.assertEqual(source_sets["external-processors"]["sourceFormat"], "platform_xml")
            self.assertEqual(source_sets["external-reports"]["kind"], "external_report")
            self.assertEqual(source_sets["external-reports"]["sourceFormat"], "platform_xml")
