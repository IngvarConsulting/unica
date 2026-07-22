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
        form_source = (
            self.repo_root()
            / fixture
            / "src"
            / "DataProcessors"
            / "ПодборСерийВДокументы"
            / "Forms"
            / "РегистрацияИПодборСерийПоОднойСтрокеТоваров"
            / "Ext"
            / "Form.xml"
        )
        self.assertNotIn("<DataPath>", form_source.read_text(encoding="utf-8"))
        form_module = form_source.parent / "Form" / "Module.bsl"
        form_module_text = form_module.read_text(encoding="utf-8")
        self.assertIn("Процедура ПриОткрытии(Отказ)", form_module_text)
        self.assertIn("Процедура ПодобратьСерии(Команда)", form_module_text)
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
            "Document.ПриобретениеТоваровУслуг",
            "Document.ПриобретениеТоваровУслуг.TabularSection.Серии",
            "Document.ПриобретениеТоваровУслуг.TabularSection.Товары",
            "Document.ПриобретениеТоваровУслуг.TabularSection.Товары.Attribute.Серия",
            "DataProcessor.ПодборСерийВДокументы",
            "DataProcessor.ПодборСерийВДокументы.Form.РегистрацияИПодборСерийПоОднойСтрокеТоваров",
        }
        candidate_targets = {candidate["target"] for candidate in first["candidates"]}
        self.assertEqual(candidate_targets, expected_targets)
        allowed_recommendation_bases = {
            "metadata_structure",
            "managed_form_binding",
            "proven_runtime_flow",
        }
        candidates_by_target = {
            candidate["target"]: candidate for candidate in first["candidates"]
        }
        for target, candidate in candidates_by_target.items():
            with self.subTest(candidate_recommendation=target):
                recommendation = candidate["recommendation"]
                self.assertTrue(recommendation["summary"].strip())
                self.assertTrue(recommendation["basis"])
                self.assertEqual(
                    len(recommendation["basis"]),
                    len(set(recommendation["basis"])),
                )
                self.assertLessEqual(
                    set(recommendation["basis"]), allowed_recommendation_bases
                )
        self.assertIn(
            "managed_form_binding",
            candidates_by_target[
                "DataProcessor.ПодборСерийВДокументы.Form."
                "РегистрацияИПодборСерийПоОднойСтрокеТоваров"
            ]["recommendation"]["basis"],
        )
        warning = next(
            warning
            for warning in first["warnings"]
            if warning["code"] == "alternative_relevant_tabular_section"
        )
        self.assertEqual(
            warning,
            {
                "code": "alternative_relevant_tabular_section",
                "message": (
                    "A point limited to a relevant nested attribute lacks coverage: "
                    "the same metadata object contains another task-relevant "
                    "tabular section."
                ),
                "blocking": True,
                "evidenceIds": [
                    "2e91ee9b968bb82f4cba4ca48770342caa68a6e79a43068a8facb4526dc08c19",
                    "b7cd2365a30c06662d052e8c12df82a01a737fcf5f49bb4ef99cd77b633092a7",
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
            "DataProcessors/ПодборСерийВДокументы/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
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
                ("/Form/Commands/Command/Action", 7, 7),
            },
        )
        series_evidence = next(
            evidence
            for evidence in first["evidence"]
            if evidence["target"]
            == "Document.ПриобретениеТоваровУслуг.TabularSection.Серии"
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
                    "rawHash": "f65dcc2a48a7e64324148a6a22bd65917f3d26be895e377f9dc86ae122fe1305",
                    "bytes": 396,
                },
                {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы/Ext/ManagerModule.bsl",
                    "rawHash": "424f23ef54e78aa09c5634041ec47d37f8a5222160ce421584b20a342adacf1b",
                    "bytes": 195,
                },
                {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров.xml",
                    "rawHash": "975b2be2f5bf8f6f61547c0cc1ad593f5be777b5dc34552fdf5f0088b3ef86ed",
                    "bytes": 277,
                },
                {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
                    "rawHash": "d534e81e93558555233e0b18dd1b9e7400c4ba8d6a345b4c0bd4968263ca658f",
                    "bytes": 303,
                },
                {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "rawHash": "c8c0c341b38cde943a60436418d182fbd523594e952f84a5965fdb8c6b3bbd2c",
                    "bytes": 849,
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
                "id": "00c68e3d90e444da1d752ed3e6e271c28079d0920c9d37e5c980a076a67e9763",
                "location": {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы.xml",
                    "line": 7,
                    "column": 7,
                    "xmlPath": "/MetaDataObject/DataProcessor/ChildObjects/Form"
                },
                "rawContentHash": "f65dcc2a48a7e64324148a6a22bd65917f3d26be895e377f9dc86ae122fe1305"
            },
            {
                "id": "1dfffb620fc00689e2ecebf399bd7c3a3a91100d3d589f6bfcc004bf4e6a2cb2",
                "location": {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров.xml",
                    "line": 2,
                    "column": 3,
                    "xmlPath": "/MetaDataObject/Form"
                },
                "rawContentHash": "975b2be2f5bf8f6f61547c0cc1ad593f5be777b5dc34552fdf5f0088b3ef86ed"
            },
            {
                "id": "24ccea36889c2a5b65316e641f7293de212adf0a1a4c53a30bf3a0b249300b67",
                "location": {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы/Ext/ManagerModule.bsl",
                    "line": 2,
                    "column": 15
                },
                "rawContentHash": "424f23ef54e78aa09c5634041ec47d37f8a5222160ce421584b20a342adacf1b"
            },
            {
                "id": "253ec2c7df7052fda499bf36ddd7ce46f422e323a4874652da975acef53a3fba",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 2
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a"
            },
            {
                "id": "2e91ee9b968bb82f4cba4ca48770342caa68a6e79a43068a8facb4526dc08c19",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "line": 12,
                    "column": 11,
                    "xmlPath": "/MetaDataObject/Document/ChildObjects/TabularSection[1]/ChildObjects/Attribute"
                },
                "rawContentHash": "c8c0c341b38cde943a60436418d182fbd523594e952f84a5965fdb8c6b3bbd2c"
            },
            {
                "id": "373f5af4c1d6833740daaf3bf655c44fb88f68a37ec2a8dc696e9984db89209b",
                "location": {
                    "relativePath": "Configuration.xml",
                    "line": 8,
                    "column": 7,
                    "xmlPath": "/MetaDataObject/Configuration/ChildObjects/DataProcessor"
                },
                "rawContentHash": "2a1d11ac5e65afe2ae599c237ef4fa78c9ebf39777e6815b09fb1f9c2f927702"
            },
            {
                "id": "48cc902e23b1566bd29cbc389f4cc736feb69c2793b6d06cdffe9b1a921dfe18",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "line": 2,
                    "column": 3,
                    "xmlPath": "/MetaDataObject/Document"
                },
                "rawContentHash": "c8c0c341b38cde943a60436418d182fbd523594e952f84a5965fdb8c6b3bbd2c"
            },
            {
                "id": "5f14e0586ccab79867afef46ada53575fd5d347d3722bec8cfebdfe20c7f1474",
                "location": {
                    "relativePath": "Configuration.xml",
                    "line": 7,
                    "column": 7,
                    "xmlPath": "/MetaDataObject/Configuration/ChildObjects/Document"
                },
                "rawContentHash": "2a1d11ac5e65afe2ae599c237ef4fa78c9ebf39777e6815b09fb1f9c2f927702"
            },
            {
                "id": "64f7b25a7f9ea2e538ff8d8b7bfcdbce3b8ea6c5e0637e1461440e2f59e4d26d",
                "location": {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
                    "line": 3,
                    "column": 5,
                    "xmlPath": "/Form/Events/Event"
                },
                "rawContentHash": "d534e81e93558555233e0b18dd1b9e7400c4ba8d6a345b4c0bd4968263ca658f"
            },
            {
                "id": "6baf2f986475dcbdcc8f6dec57026024ee8a00bf7e7ffa82c549d0baa60ffbc1",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 2
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a"
            },
            {
                "id": "99b5178d4e4615b5df3767eea5256f3d4d573fbfcc11914bc6742b8a3ee2ebfd",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "line": 7,
                    "column": 7,
                    "xmlPath": "/MetaDataObject/Document/ChildObjects/TabularSection[1]"
                },
                "rawContentHash": "c8c0c341b38cde943a60436418d182fbd523594e952f84a5965fdb8c6b3bbd2c"
            },
            {
                "id": "9a8cf55287152c9deb5d20b83f2d38d26fdf7ad29988c4d681e10904b40cc804",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 2
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a"
            },
            {
                "id": "b7cd2365a30c06662d052e8c12df82a01a737fcf5f49bb4ef99cd77b633092a7",
                "location": {
                    "relativePath": "Documents/ПриобретениеТоваровУслуг.xml",
                    "line": 19,
                    "column": 7,
                    "xmlPath": "/MetaDataObject/Document/ChildObjects/TabularSection[2]"
                },
                "rawContentHash": "c8c0c341b38cde943a60436418d182fbd523594e952f84a5965fdb8c6b3bbd2c"
            },
            {
                "id": "b907eedfbce198e7f7e26d288913beac37ca626ac657d76ccf282c96148db23a",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 16
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a"
            },
            {
                "id": "bbfc19ea78b2cae1a007d085225bb72ab3b0b2255623eeb05becac5278332b0c",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 12
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a"
            },
            {
                "id": "bd11100ea389aa47ff7e781bb117528e9b1b868d430a4e69aaede15828c709e7",
                "location": {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
                    "line": 7,
                    "column": 7,
                    "xmlPath": "/Form/Commands/Command/Action"
                },
                "rawContentHash": "d534e81e93558555233e0b18dd1b9e7400c4ba8d6a345b4c0bd4968263ca658f"
            },
            {
                "id": "e724f323907dbe2bf13ba2d274364db104c577f40b1d50e70232e68f5a37960e",
                "location": {
                    "relativePath": "DataProcessors/ПодборСерийВДокументы.xml",
                    "line": 2,
                    "column": 3,
                    "xmlPath": "/MetaDataObject/DataProcessor"
                },
                "rawContentHash": "f65dcc2a48a7e64324148a6a22bd65917f3d26be895e377f9dc86ae122fe1305"
            },
            {
                "id": "e749078c33c5eda3d989a416e0196551a975b4523f2c7aef213a0fb8d357cb31",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 2
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a"
            },
            {
                "id": "f4a33542a429c060d669a73cd26cc0d725b81c5fedf83aa959dec0b6eb6df5bf",
                "location": {
                    "relativePath": "Configuration.xml",
                    "line": 2,
                    "column": 3,
                    "xmlPath": "/MetaDataObject/Configuration"
                },
                "rawContentHash": "2a1d11ac5e65afe2ae599c237ef4fa78c9ebf39777e6815b09fb1f9c2f927702"
            },
            {
                "id": "faa0e205576b8484221676d406d797355edd5552d7f370831006fa8799ae7200",
                "location": {
                    "relativePath": "Ext/ParentConfigurations.bin",
                    "line": 2
                },
                "rawContentHash": "696c7c22cf43a508b281104e6f03060980c1e496e094c91783db843d09d80c2a"
            }
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
            first["analysisSnapshot"]["mappingFingerprint"],
            "5d4261cfbf2df1cebba21cd0c624fceedab610777baa8dceaabb7c8f314a2b74",
        )
        self.assertEqual(
            first["analysisSnapshot"]["fingerprint"],
            "054912abd5451cfce3eb6eea4d3916460d93f89e3a8ef5636e2e7d009dbd08e8",
        )
        self.assertEqual(
            second["analysisSnapshot"]["mappingFingerprint"],
            first["analysisSnapshot"]["mappingFingerprint"],
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
