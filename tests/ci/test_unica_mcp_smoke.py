from __future__ import annotations

import hashlib
import json
import os
import queue
import subprocess
import tempfile
import threading
import time
import unittest
from pathlib import Path


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
            return responses
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
        self.assertIn("unica.epf.init", tools)
        self.assertIn("unica.erf.init", tools)
        self.assertIn("unica.build.load", tools)
        self.assertIn("unica.runtime.execute", tools)
        self.assertIn("unica.standards.explain", tools)

    def test_task_only_extension_point_discovery_returns_stable_ut115_evidence(self) -> None:
        fixture = Path("tests/fixtures/extension-point-discovery/ut115")
        arguments = {
            "cwd": str(fixture),
            "mode": "explore",
            "task": "При поступлении товаров контролировать остаточный срок годности серий",
        }
        with tempfile.TemporaryDirectory() as tmp:
            responses = self.call_mcp(
                [
                    {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.project.discover",
                            "arguments": arguments,
                        },
                    }
                    for request_id in (1, 2)
                ],
                cache_dir=Path(tmp) / "cache",
            )

        payloads = {
            response["id"]: json.loads(response["result"]["content"][0]["text"])
            for response in responses
        }
        first = payloads[1]["data"]["discovery"]
        second = payloads[2]["data"]["discovery"]

        self.assertNotIn("stdout", payloads[1])
        self.assertEqual(payloads[1]["changes"], [])
        self.assertEqual(payloads[1]["cache"]["mode"], "read")
        self.assertEqual(payloads[1]["cache"]["events"], [])
        self.assertEqual(payloads[1]["cache"]["invalidated"], [])
        self.assertEqual(payloads[1]["cache"]["refreshed"], [])
        self.assertEqual(payloads[1]["cache"]["lazy_rebuilt"], [])
        self.assertEqual(first["status"], "partial")
        outcomes = {
            outcome["provider"]: outcome for outcome in first["providerOutcomes"]
        }
        self.assertEqual(outcomes["metadata_catalog"]["outcome"], "complete")
        self.assertEqual(outcomes["managed_forms"]["outcome"], "complete")
        missing_codes = {check["code"] for check in first["missingChecks"]}
        self.assertIn("bsl_index_missing", missing_codes)
        self.assertIn("runtime_flow_unavailable", missing_codes)
        self.assertFalse(
            any(edge["relation"] == "calls" for edge in first["runtimeFlowEdges"]),
            "lexical BSL evidence must not be promoted to a call-graph edge",
        )

        required_targets = {
            "document.приобретениетоваровуслуг.tabularsection.серии",
            "dataprocessor.подборсерийвдокументы",
            "document.приобретениетоваровуслуг.form.регистрацияиподборсерийпооднойстрокетоваров",
        }
        candidate_targets = {candidate["target"] for candidate in first["candidates"]}
        self.assertTrue(required_targets <= candidate_targets, candidate_targets)
        warning = next(
            warning
            for warning in first["warnings"]
            if warning["code"] == "separate_series_section"
        )
        self.assertTrue(warning["blocking"])
        self.assertIn("Товары.Серия", warning["message"])

        evidence_kinds = {evidence["kind"] for evidence in first["evidence"]}
        self.assertIn("metadata", evidence_kinds)
        self.assertIn("form_binding", evidence_kinds)
        locations = {
            evidence["location"]["relativePath"] for evidence in first["evidence"]
        }
        self.assertIn("Documents/ПриобретениеТоваровУслуг.xml", locations)
        self.assertIn(
            "Documents/ПриобретениеТоваровУслуг/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
            locations,
        )
        self.assertIn(
            "DataProcessors/ПодборСерийВДокументы/Ext/ManagerModule.bsl",
            locations,
        )
        form_locations = {
            (
                evidence["location"]["xmlPath"],
                evidence["location"]["line"],
                evidence["location"]["column"],
            )
            for evidence in first["evidence"]
            if evidence["kind"] == "form_binding"
        }
        self.assertEqual(
            form_locations,
            {
                ("/Form/Events/Event", 3, 5),
                ("/Form/ChildItems/InputField/DataPath", 7, 7),
                ("/Form/Commands/Command/Action", 12, 7),
            },
        )
        series_evidence = next(
            evidence
            for evidence in first["evidence"]
            if evidence["target"]
            == "document.приобретениетоваровуслуг.tabularsection.серии"
        )
        self.assertEqual(series_evidence["location"]["line"], 19)
        self.assertEqual(
            series_evidence["location"]["xmlPath"],
            "/MetaDataObject/Document/ChildObjects/TabularSection[2]",
        )
        bsl_evidence = next(
            evidence for evidence in first["evidence"] if evidence["kind"] == "lexical"
        )
        self.assertEqual(bsl_evidence["location"]["line"], 2)

        fixture_root = self.repo_root() / fixture / "src"
        contributors = first["analysisSnapshot"]["contributors"]
        contributor_hashes = {
            contributor["relativePath"]: contributor["rawHash"]
            for contributor in contributors
        }
        self.assertEqual(
            set(contributor_hashes),
            {
                "Configuration.xml",
                "DataProcessors/ПодборСерийВДокументы.xml",
                "DataProcessors/ПодборСерийВДокументы/Ext/ManagerModule.bsl",
                "Documents/ПриобретениеТоваровУслуг.xml",
                "Documents/ПриобретениеТоваровУслуг/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров.xml",
                "Documents/ПриобретениеТоваровУслуг/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
                "Ext/ParentConfigurations.bin",
            },
        )
        for relative_path, raw_hash in contributor_hashes.items():
            expected = hashlib.sha256((fixture_root / relative_path).read_bytes()).hexdigest()
            self.assertEqual(raw_hash, expected, relative_path)
        for evidence in first["evidence"]:
            relative_path = evidence["location"]["relativePath"]
            expected = hashlib.sha256((fixture_root / relative_path).read_bytes()).hexdigest()
            self.assertEqual(evidence["rawContentHash"], expected, relative_path)

        self.assertEqual(
            first["analysisSnapshot"]["fingerprint"],
            second["analysisSnapshot"]["fingerprint"],
        )
        self.assertEqual(
            [evidence["id"] for evidence in first["evidence"]],
            [evidence["id"] for evidence in second["evidence"]],
        )
        serialized = json.dumps(first, ensure_ascii=False)
        self.assertNotIn("ExactExpectedNames", serialized)

    def test_notifications_do_not_count_as_responses(self) -> None:
        responses = self.call_mcp(
            [
                {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
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
