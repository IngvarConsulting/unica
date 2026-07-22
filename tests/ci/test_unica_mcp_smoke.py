from __future__ import annotations

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
            cache_dir = Path(tmp) / "cache"
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
                cache_dir=cache_dir,
            )
            cache_residual = list(cache_dir.rglob("*")) if cache_dir.exists() else []
            self.assertEqual(cache_residual, [], cache_residual)

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

        expected_targets = {
            "document.приобретениетоваровуслуг",
            "document.приобретениетоваровуслуг.tabularsection.серии",
            "document.приобретениетоваровуслуг.tabularsection.товары",
            "document.приобретениетоваровуслуг.tabularsection.товары.attribute.серия",
            "dataprocessor.подборсерийвдокументы",
            "document.приобретениетоваровуслуг.form.регистрацияиподборсерийпооднойстрокетоваров",
        }
        candidate_targets = {candidate["target"] for candidate in first["candidates"]}
        self.assertEqual(candidate_targets, expected_targets)
        warning = next(
            warning
            for warning in first["warnings"]
            if warning["code"] == "separate_series_section"
        )
        self.assertEqual(
            warning,
            {
                "code": "separate_series_section",
                "message": (
                    "A point limited to Товары.Серия lacks coverage: the same relevant "
                    "document contains a distinct series-related tabular section."
                ),
                "blocking": True,
                "evidenceIds": [
                    "1cd2a4e528117a3b288f6e0774d007d34e6c44fcb1107a43710e32cf444e6a4d",
                    "975181c0218292f4f50500f0b4fe72a48f047a88032e478f1521a11d0ecedf40",
                ],
            },
        )

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

        contributors = first["analysisSnapshot"]["contributors"]
        self.assertEqual(
            contributors,
            [
                {
                    "relativePath": "Configuration.xml",
                    "rawHash": "2a1d11ac5e65afe2ae599c237ef4fa78c9ebf39777e6815b09fb1f9c2f927702",
                    "bytes": 409,
                },
                {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы.xml",
                    "rawHash": "fe394f08071bbd84f5298c50d11c88c456ef59b91c380832219de4d176c10d29",
                    "bytes": 251,
                },
                {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы/Ext/ManagerModule.bsl",
                    "rawHash": "424f23ef54e78aa09c5634041ec47d37f8a5222160ce421584b20a342adacf1b",
                    "bytes": 195,
                },
                {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "rawHash": "c107c0ab9f5b429ee61f8838780dfef4caf6e7753268461c3770dbed5431bbad",
                    "bytes": 955,
                },
                {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров.xml",
                    "rawHash": "975b2be2f5bf8f6f61547c0cc1ad593f5be777b5dc34552fdf5f0088b3ef86ed",
                    "bytes": 277,
                },
                {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
                    "rawHash": "049d0a56fc6f06703821a3dfbc290ea67bb5462ea44f7871b372ee2a30331a61",
                    "bytes": 470,
                },
                {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "rawHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a",
                    "bytes": 284,
                },
            ],
        )

        expected_evidence_identity = [
            {
                "id": "033f28d52b17975e01f8d3e4e76368b231e551d25b1825d3de75be90435edcc7",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
                    "line": 3,
                    "column": 5,
                    "xmlPath": "/Form/Events/Event",
                },
                "rawContentHash": "049d0a56fc6f06703821a3dfbc290ea67bb5462ea44f7871b372ee2a30331a61",
            },
            {
                "id": "1cd2a4e528117a3b288f6e0774d007d34e6c44fcb1107a43710e32cf444e6a4d",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "line": 12,
                    "column": 11,
                    "xmlPath": "/MetaDataObject/Document/ChildObjects/TabularSection[1]/ChildObjects/Attribute",
                },
                "rawContentHash": "c107c0ab9f5b429ee61f8838780dfef4caf6e7753268461c3770dbed5431bbad",
            },
            {
                "id": "2005aa4bd315cba6fdebd54d4733cc01a1d2d50115119b7fe0a190a90db2e332",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
                    "line": 12,
                    "column": 7,
                    "xmlPath": "/Form/Commands/Command/Action",
                },
                "rawContentHash": "049d0a56fc6f06703821a3dfbc290ea67bb5462ea44f7871b372ee2a30331a61",
            },
            {
                "id": "24ccea36889c2a5b65316e641f7293de212adf0a1a4c53a30bf3a0b249300b67",
                "location": {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы/Ext/ManagerModule.bsl",
                    "line": 2,
                    "column": 15,
                },
                "rawContentHash": "424f23ef54e78aa09c5634041ec47d37f8a5222160ce421584b20a342adacf1b",
            },
            {
                "id": "253ec2c7df7052fda499bf36ddd7ce46f422e323a4874652da975acef53a3fba",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 2,
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a",
            },
            {
                "id": "373f5af4c1d6833740daaf3bf655c44fb88f68a37ec2a8dc696e9984db89209b",
                "location": {
                    "relativePath": "Configuration.xml",
                    "line": 8,
                    "column": 7,
                    "xmlPath": "/MetaDataObject/Configuration/ChildObjects/DataProcessor",
                },
                "rawContentHash": "2a1d11ac5e65afe2ae599c237ef4fa78c9ebf39777e6815b09fb1f9c2f927702",
            },
            {
                "id": "48c118bd9d715fe45d507f07d74c6b533fdba58ffcdd2fb5b1e4f831f403f16e",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "line": 24,
                    "column": 7,
                    "xmlPath": "/MetaDataObject/Document/ChildObjects/Form",
                },
                "rawContentHash": "c107c0ab9f5b429ee61f8838780dfef4caf6e7753268461c3770dbed5431bbad",
            },
            {
                "id": "548b7db14fc2d29475fbefd37f4bc13e53de9218fe54a714b00a1bb92b2c11a5",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "line": 2,
                    "column": 3,
                    "xmlPath": "/MetaDataObject/Document",
                },
                "rawContentHash": "c107c0ab9f5b429ee61f8838780dfef4caf6e7753268461c3770dbed5431bbad",
            },
            {
                "id": "54ba130bf9d59d04e4c5b163244e8fd2b3b40ba15328cfd02a1e79eb07cfa1a2",
                "location": {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы.xml",
                    "line": 2,
                    "column": 3,
                    "xmlPath": "/MetaDataObject/DataProcessor",
                },
                "rawContentHash": "fe394f08071bbd84f5298c50d11c88c456ef59b91c380832219de4d176c10d29",
            },
            {
                "id": "5f14e0586ccab79867afef46ada53575fd5d347d3722bec8cfebdfe20c7f1474",
                "location": {
                    "relativePath": "Configuration.xml",
                    "line": 7,
                    "column": 7,
                    "xmlPath": "/MetaDataObject/Configuration/ChildObjects/Document",
                },
                "rawContentHash": "2a1d11ac5e65afe2ae599c237ef4fa78c9ebf39777e6815b09fb1f9c2f927702",
            },
            {
                "id": "6baf2f986475dcbdcc8f6dec57026024ee8a00bf7e7ffa82c549d0baa60ffbc1",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 2,
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a",
            },
            {
                "id": "6bda895d8a458ab1733087bfd85c3ad1020f69e03da46bd731f9677f7600c295",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров.xml",
                    "line": 2,
                    "column": 3,
                    "xmlPath": "/MetaDataObject/Form",
                },
                "rawContentHash": "975b2be2f5bf8f6f61547c0cc1ad593f5be777b5dc34552fdf5f0088b3ef86ed",
            },
            {
                "id": "7beea8cd4895cd07c1643396f269c6ce8774f0837effb3d565cb1295c9301442",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
                    "line": 7,
                    "column": 7,
                    "xmlPath": "/Form/ChildItems/InputField/DataPath",
                },
                "rawContentHash": "049d0a56fc6f06703821a3dfbc290ea67bb5462ea44f7871b372ee2a30331a61",
            },
            {
                "id": "975181c0218292f4f50500f0b4fe72a48f047a88032e478f1521a11d0ecedf40",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "line": 19,
                    "column": 7,
                    "xmlPath": "/MetaDataObject/Document/ChildObjects/TabularSection[2]",
                },
                "rawContentHash": "c107c0ab9f5b429ee61f8838780dfef4caf6e7753268461c3770dbed5431bbad",
            },
            {
                "id": "994c8a9d6c5a4aff5157e8a1ce3918853fc76c1737de84a5772b7a5e24e3bf6d",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "line": 7,
                    "column": 7,
                    "xmlPath": "/MetaDataObject/Document/ChildObjects/TabularSection[1]",
                },
                "rawContentHash": "c107c0ab9f5b429ee61f8838780dfef4caf6e7753268461c3770dbed5431bbad",
            },
            {
                "id": "9a8cf55287152c9deb5d20b83f2d38d26fdf7ad29988c4d681e10904b40cc804",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 2,
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a",
            },
            {
                "id": "a0806b98d993150e8bba41c2f67b0893e18690e27d4d72ce0c2b98d4da22a550",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 2,
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a",
            },
            {
                "id": "b907eedfbce198e7f7e26d288913beac37ca626ac657d76ccf282c96148db23a",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 16,
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a",
            },
            {
                "id": "bbfc19ea78b2cae1a007d085225bb72ab3b0b2255623eeb05becac5278332b0c",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 12,
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a",
            },
            {
                "id": "e749078c33c5eda3d989a416e0196551a975b4523f2c7aef213a0fb8d357cb31",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 2,
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a",
            },
            {
                "id": "f4a33542a429c060d669a73cd26cc0d725b81c5fedf83aa959dec0b6eb6df5bf",
                "location": {
                    "relativePath": "Configuration.xml",
                    "line": 2,
                    "column": 3,
                    "xmlPath": "/MetaDataObject/Configuration",
                },
                "rawContentHash": "2a1d11ac5e65afe2ae599c237ef4fa78c9ebf39777e6815b09fb1f9c2f927702",
            },
        ]
        actual_evidence_identity = [
            {
                "id": evidence["id"],
                "location": evidence["location"],
                "rawContentHash": evidence["rawContentHash"],
            }
            for evidence in first["evidence"]
        ]
        self.assertEqual(actual_evidence_identity, expected_evidence_identity)

        self.assertEqual(
            first["analysisSnapshot"]["fingerprint"],
            "cdcb541428fe80db6733d5ab97afab4d29b27246ae420d9aa322eb0a029c4f2a",
        )
        self.assertEqual(
            second["analysisSnapshot"]["fingerprint"],
            first["analysisSnapshot"]["fingerprint"],
        )
        self.assertEqual(
            [evidence["id"] for evidence in second["evidence"]],
            [item["id"] for item in expected_evidence_identity],
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
