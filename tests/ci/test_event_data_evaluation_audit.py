"""Protocol counterexamples for the read-only event visibility evaluation."""
import copy
import unittest
import json
import tempfile
from pathlib import Path
from unittest.mock import patch

from tests.agent_evaluation import event_data_driver as driver


def transcript():
    records = []
    files = {"v8project.yaml": "project", driver.SOURCE_FILE: "module"}
    observation = {"sourceSha256": "source", "sourceFilesSha256": files,
                   "sourceSetRoots": {"main": "src"}}
    def exchange(method, params, result):
        identity = len(records)
        records.append({"direction": "request", **copy.deepcopy(observation),
                        "payload": {"id": identity, "method": method, "params": params}})
        records.append({"direction": "response", **copy.deepcopy(observation),
                        "payload": {"id": identity, "result": result}})
    exchange("tools/list", {}, {"tools": [{"name": n} for n in
             ("unica.view", "unica.docs", "unica.task.result", "unica.apply")]})
    exchange("tools/call", {"name": "unica.view", "arguments": {"at": driver.MODULE}},
             {"structuredContent": {"ok": True, "data": {"text":
              (driver.SOURCE_FIXTURE / driver.SOURCE_FILE).read_text()}}})
    for document_id in driver.DOCUMENTS.values():
        exchange("tools/call", {"name": "unica.docs", "arguments": {"query": document_id}},
                 {"structuredContent": {"ok": True, "data": {"document": {
                     "documentId": document_id, "text": "synthetic page content",
                     "sourceKind": "platform-help", "authority": "vendor",
                     "applicableVersion": "8.3.27.2214"}}}})
    return records


class EventDataEvidenceAuditTests(unittest.TestCase):
    def test_reviewer_receives_observed_skill_reads_not_just_transcript_hash(self):
        with tempfile.TemporaryDirectory() as temp:
            proof = Path(temp)
            read = {"type": "command_execution", "command": "cat " + str(proof / "plugin" / driver.HELP_REFS[0]),
                    "aggregated_output": "candidate skill contents", "exit_code": 0, "status": "completed"}
            unrelated = {"type": "reasoning", "text": "unrelated reasoning"}
            (proof / "agent.jsonl").write_text("\n".join(json.dumps({"type": "item.completed", "item": item})
                                                        for item in (read, unrelated)))
            with patch.object(driver, "review_contract") as review:
                driver.independent_review(proof)
                review.assert_called_once_with(proof, rubric=driver.RUBRIC)
            observations = json.loads((proof / "file-observations.json").read_text())
            self.assertEqual(observations, {"helpReads": [read]})

    def test_complete_read_only_evidence_is_accepted(self):
        _, calls = driver.audit(transcript())
        self.assertEqual(len(calls), 4)

    def test_literal_body_content_is_read_without_structured_content(self):
        records = transcript()
        original = records[3]["payload"]["result"].pop("structuredContent")
        import json
        records[3]["payload"]["result"]["content"] = [
            {"type": "text", "text": json.dumps({"ok": True, "data": {
                "kind": "Body", "at": driver.MODULE + ".Body"}})},
            {"type": "text", "text": original["data"]["text"]}]
        _, calls = driver.audit(records)
        self.assertEqual(len(calls), 4)
        records[3]["payload"]["result"]["content"].pop()
        with self.assertRaisesRegex(ValueError, "complete synthetic"):
            driver.audit(records)

    def test_missing_or_partial_module_is_rejected(self):
        for text in ("", "Процедура ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)"):
            with self.subTest(text=text), self.assertRaisesRegex(ValueError, "complete synthetic"):
                records = transcript()
                records[3]["payload"]["result"]["structuredContent"]["data"]["text"] = text
                driver.audit(records)

    def test_search_snippets_cannot_replace_opened_pages(self):
        records = transcript()
        data = records[5]["payload"]["result"]["structuredContent"]["data"]
        data["hits"] = [data.pop("document")]
        with self.assertRaisesRegex(ValueError, "open the event"):
            driver.audit(records)

    def test_wrong_version_authority_and_incomplete_page_are_rejected(self):
        for key, value in (("applicableVersion", "8.5.1.1529"), ("authority", "community"),
                           ("sourceKind", "standards"), ("text", "")):
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "full vendor pages"):
                records = transcript()
                records[5]["payload"]["result"]["structuredContent"]["data"]["document"][key] = value
                driver.audit(records)
        records = transcript()
        records[5]["payload"]["result"]["structuredContent"]["cursor"] = "next"
        with self.assertRaisesRegex(ValueError, "full vendor pages"):
            driver.audit(records)

    def test_source_or_map_mutation_and_missing_observations_are_rejected(self):
        for key, value in (("sourceSha256", "changed"), ("sourceFilesSha256", {}),
                           ("sourceSetRoots", {"other": "src"})):
            with self.subTest(key=key), self.assertRaises(ValueError):
                records = transcript()
                records[3][key] = value
                driver.audit(records)
        records = transcript()
        for record in records:
            record["sourceFilesSha256"].pop("v8project.yaml")
        with self.assertRaisesRegex(ValueError, "omitted source/project"):
            driver.audit(records)

    def test_completed_task_keeps_both_calls_for_cli_reconciliation(self):
        records = transcript()
        original = records[5]["payload"]["result"]["structuredContent"]
        records[5]["payload"]["result"]["structuredContent"] = {
            "ok": True, "data": {"task": {"taskId": "async", "status": "working"}}}
        request, response = copy.deepcopy(records[4:6])
        request["payload"]["id"] = response["payload"]["id"] = 100
        request["payload"]["params"] = {"name": "unica.task.result", "arguments": {"taskId": "async"}}
        response["payload"]["result"]["structuredContent"] = original
        records[6:6] = [request, response]
        _, calls = driver.audit(records)
        self.assertEqual(len(calls), 5)

    def test_mutation_tool_and_unfinished_task_are_rejected(self):
        records = transcript()
        records[2]["payload"]["params"]["name"] = "unica.apply"
        with self.assertRaisesRegex(ValueError, "outside read-only"):
            driver.audit(records)
        records = transcript()
        records[5]["payload"]["result"]["structuredContent"]["data"] = {
            "task": {"taskId": "pending", "status": "working"}}
        with self.assertRaisesRegex(ValueError, "never reached"):
            driver.audit(records)
