"""A forbidden evidence path cannot become a passing agent evaluation."""
import unittest
from pathlib import Path

from tests.agent_evaluation.dcs_driver import DCS_TEMPLATE, allowed_help_command, audit_cli, audit_mcp, audit_review, terminal_calls


class AgentEvaluationAuditTests(unittest.TestCase):
    def test_failed_reread_or_check_of_other_node_cannot_prove_agent_verification(self):
        def records(read_ok=True, check_at=DCS_TEMPLATE):
            trace = []
            digest = "old"
            def exchange(method, params, result):
                nonlocal digest
                identifier = len(trace)
                trace.append({"direction": "request", "sourceSha256": digest,
                              "payload": {"id": identifier, "method": method, "params": params}})
                if result.get("data", {}).get("mode") == "published":
                    digest = "new"
                trace.append({"direction": "response", "sourceSha256": digest,
                              "payload": {"id": identifier, "result": (
                                  result if method == "tools/list" else {"structuredContent": result})}})
            exchange("tools/list", {}, {"tools": [{"name": name} for name in ["unica.view", "unica.apply", "unica.check"]]})
            operations = []
            for op, suffix in [("field.add", ".DataSet.F05Data"), ("query.set", ".DataSet.F05Data"),
                               ("structure.patch", ".Setting.F05Variant")]:
                target = DCS_TEMPLATE + suffix
                exchange("tools/call", {"name": "unica.view", "arguments": {"at": target}}, {
                    "ok": True, "data": {"can": [{"op": op, "implemented": True, "contract": {"argsSchema": {"type": "object"}}}]}})
                operations.append({"op": op, "args": {"at": target}})
            exchange("tools/call", {"name": "unica.apply", "arguments": {"at": DCS_TEMPLATE, "ops": operations}},
                     {"ok": True, "data": {"mode": "preview", "effects": 1, "executionToken": "p"}})
            exchange("tools/call", {"name": "unica.apply", "arguments": {"executionToken": "p"}},
                     {"ok": True, "data": {"mode": "published", "effects": 1}})
            exchange("tools/call", {"name": "unica.view", "arguments": {"at": DCS_TEMPLATE}}, {"ok": read_ok})
            exchange("tools/call", {"name": "unica.check", "arguments": {"at": check_at}},
                     {"ok": True, "data": {"status": "passed"}})
            return trace
        audit_mcp(records())
        with self.assertRaisesRegex(ValueError, "reread"):
            audit_mcp(records(read_ok=False))
        with self.assertRaisesRegex(ValueError, "check"):
            audit_mcp(records(check_at="cf:Configuration"))

    def test_only_literal_cat_of_prepared_help_is_allowed(self):
        path = Path("/prepared/plugin/reports printing.md")
        self.assertTrue(allowed_help_command("/bin/zsh -lc \"cat '/prepared/plugin/reports printing.md'\"", path))
        for command in ["cat /other/skill.md", "cat '/prepared/plugin/reports printing.md'; cat /old/skill.md",
                        "python -c 'print(open(\"/old/skill.md\").read())'", "cat $(echo /prepared/plugin/reports\\ printing.md)",
                        "cat '/prepared/plugin/reports printing.md' > /workspace/template.xml"]:
            with self.subTest(command=command):
                self.assertFalse(allowed_help_command(command, path))

    def test_unknown_failed_and_tool_events_refuse_independent_review(self):
        for event in [{"type": "error"}, {"type": "turn.failed"}, {"type": "future.event"},
                      {"type": "item.started", "item": {"type": "mcp_tool_call"}},
                      {"type": "item.completed", "item": {"type": "error"}}]:
            with self.subTest(event=event), self.assertRaises(ValueError):
                audit_review([{"type": "turn.completed"}, event])
        for events in [[], [{"type": "turn.completed"}] * 2]:
            with self.assertRaises(ValueError):
                audit_review(events)

    def test_arbitrary_shell_and_other_mcp_cannot_pass_the_agent_audit(self):
        for item in [{"type": "command_execution", "command": "cat /old/skill.md"},
                     {"type": "mcp_tool_call", "server": "other", "tool": "unica.view"},
                     {"type": "unknown_tool"}]:
            with self.subTest(item=item), self.assertRaises(ValueError):
                audit_cli([{"type": "item.started", "item": item}], {"unica.view"}, Path("/prepared/help.md"))

    def test_task_get_observes_the_originating_task_until_original_result(self):
        def call(index, name, args, result):
            return {"index": index, "done": index + 1, "params": {"name": name, "arguments": args},
                    "result": result, "before": "old", "after": "new"}
        queued = {"ok": True, "data": {"task": {"taskId": "t", "status": "queued"}}}
        working = {"ok": True, "data": {"task": {"taskId": "t", "status": "working"}}}
        published = {"ok": True, "data": {"mode": "published", "effects": 1}}
        origin = call(0, "unica.apply", {"executionToken": "observed-preview"}, queued)
        observation = call(2, "unica.task.get", {"taskId": "t"}, working)
        result = call(4, "unica.task.result", {"taskId": "t"}, published)
        finished, = terminal_calls([origin, observation, result])
        self.assertEqual(finished["params"], origin["params"])
        self.assertEqual(finished["result"], published)
        self.assertEqual(finished["done"], 5)
        observation["result"] = {"ok": True, "data": {"task": {"taskId": "other", "status": "working"}}}
        with self.assertRaisesRegex(ValueError, "identity"):
            terminal_calls([origin, observation, result])
        with self.assertRaisesRegex(ValueError, "terminal"):
            terminal_calls([origin])
