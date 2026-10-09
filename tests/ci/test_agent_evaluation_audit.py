"""A forbidden evidence path cannot become a passing agent evaluation."""
import unittest
import tempfile
import json
import hashlib
import os
from unittest.mock import patch
from pathlib import Path

from tests.agent_evaluation.dcs_driver import DCS_TEMPLATE, allowed_help_command, audit_cli, audit_mcp, audit_review, terminal_calls


class AgentEvaluationAuditTests(unittest.TestCase):
    def test_creation_recorder_observes_owner_peers_and_new_files(self):
        from tests.agent_evaluation.mcp_recorder import source_digest
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw); source = root / "src"; source.mkdir()
            owner = source / "owner.xml"; owner.write_text("before")
            before = source_digest(source)
            owner.write_text("registered")
            self.assertNotEqual(source_digest(source), before)
            registered = source_digest(source)
            layout = source / "Template.xml"; layout.write_text("content")
            self.assertNotEqual(source_digest(source), registered)
            published = source_digest(source)
            layout.rename(source / "Other.xml")
            self.assertNotEqual(source_digest(source), published, "names are part of the source identity")
            (source / "link").symlink_to(root / "missing")
            with self.assertRaisesRegex(ValueError,"symlinks"):
                source_digest(source)

    def test_cleanup_accepts_a_terminated_daemon_and_never_signals_a_reused_pid(self):
        import signal
        import subprocess
        from tests.agent_evaluation.dcs_driver import stop_owned_daemons
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            endpoint = state / "daemon-p5-test/endpoint.json"
            endpoint.parent.mkdir()
            endpoint.write_text(json.dumps({"pid": 42}))
            binary = state / "plugin/bin/unica"
            owned = f"{binary} --daemon --state-root {state}"
            for terminal in ("Z <defunct>", "S /unrelated/process"):
                observations = iter([f"S {owned}", terminal])
                def observe(command, **kwargs):
                    observation = next(observations, terminal)
                    if command[-1] == "args=":
                        observation = observation.split(" ", 1)[1]
                    return subprocess.CompletedProcess(command, 0, observation, "")
                with self.subTest(terminal=terminal), \
                     patch("tests.agent_evaluation.dcs_driver.subprocess.run", side_effect=observe), \
                     patch("tests.agent_evaluation.dcs_driver.os.kill") as kill:
                    stop_owned_daemons(state, binary)
                    kill.assert_called_once_with(42, signal.SIGTERM)
            with patch("tests.agent_evaluation.dcs_driver.subprocess.run", return_value=
                       subprocess.CompletedProcess([], 0, "S /unrelated/process", "")), \
                 patch("tests.agent_evaluation.dcs_driver.os.kill") as kill:
                with self.assertRaisesRegex(ValueError, "no longer belongs"):
                    stop_owned_daemons(state, binary)
                kill.assert_not_called()

    def test_packet_uses_the_same_binary_as_the_verifying_server(self):
        from tests.agent_evaluation.dcs_driver import prepare_packet
        binary_name = "unica.exe" if os.name == "nt" else "unica"
        class Builder:
            @staticmethod
            def copy_tracked_plugin_source(repo, source, packet):
                packet.mkdir()
            @staticmethod
            def assert_host_manifests_present(packet):
                pass
            @staticmethod
            def write_local_debug_mcp_launcher(packet, target, host):
                command = f"./bin/test/{binary_name}" if host == "codex" else f"${{CLAUDE_PLUGIN_ROOT}}/bin/test/{binary_name}"
                (packet / ".mcp.json").write_text(json.dumps({"mcpServers": {"unica": {
                    "command": command, "env": {"UNICA_HOST_CONTEXT_REQUIRED": "1"}}}}))
            @staticmethod
            def package_tree_sha256(packet):
                return hashlib.sha256((packet / "bin/test" / binary_name).read_bytes()).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "target/debug").mkdir(parents=True)
            (root / "target/debug/unica").write_bytes(b"stale-default-binary")
            (root / "plugins/unica/third-party").mkdir(parents=True)
            (root / "plugins/unica/third-party/tools.lock.json").write_text("{}")
            current = root / "custom-target/debug/unica"
            current.parent.mkdir(parents=True)
            current.write_bytes(b"actual-server-binary")
            with patch("tests.agent_evaluation.dcs_driver.REPO", root), \
                 patch("tests.agent_evaluation.dcs_driver.host_target", return_value="test"), \
                 patch("tests.agent_evaluation.dcs_driver.packager", return_value=Builder):
                _, copied, digest = prepare_packet(root, current)
            self.assertEqual(copied.read_bytes(), current.read_bytes())
            self.assertEqual(digest, hashlib.sha256(current.read_bytes()).hexdigest())

    def test_failed_reread_or_check_of_other_node_cannot_prove_agent_verification(self):
        def records(read_ok=True, check_at=DCS_TEMPLATE, discarded_preview=False, omit_op=None):
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
            if discarded_preview:
                exchange("tools/call", {"name": "unica.apply", "arguments": {"at": DCS_TEMPLATE, "ops": operations}},
                         {"ok": True, "data": {"mode": "preview", "effects": 1, "executionToken": "discarded"}})
            selected = [op for op in operations if op["op"] != omit_op]
            exchange("tools/call", {"name": "unica.apply", "arguments": {"at": DCS_TEMPLATE, "ops": selected}},
                     {"ok": True, "data": {"mode": "preview", "effects": 1, "executionToken": "p"}})
            exchange("tools/call", {"name": "unica.apply", "arguments": {"executionToken": "p"}},
                     {"ok": True, "data": {"mode": "published", "effects": 1}})
            exchange("tools/call", {"name": "unica.view", "arguments": {"at": DCS_TEMPLATE}}, {"ok": read_ok})
            exchange("tools/call", {"name": "unica.check", "arguments": {"at": check_at}},
                     {"ok": True, "data": {"status": "passed"}})
            return trace
        audit_mcp(records())
        audit_mcp(records(discarded_preview=True))
        with self.assertRaisesRegex(ValueError, "requested field, query and grouping effects"):
            audit_mcp(records(discarded_preview=True, omit_op="query.set"))
        with self.assertRaisesRegex(ValueError, "reread"):
            audit_mcp(records(read_ok=False))
        with self.assertRaisesRegex(ValueError, "check"):
            audit_mcp(records(check_at="cf:Configuration"))

    def test_mxl_creation_audit_rejects_foreign_owner_unobserved_token_and_writing_preview(self):
        from tests.agent_evaluation.mxl_driver import OWNER, TEMPLATE, audit
        import copy
        trace = []
        digest = "original-tree"
        def exchange(name, args, result):
            nonlocal digest
            identifier = len(trace)
            trace.append({"direction": "request", "sourceSha256": digest,
                          "payload": {"id": identifier, "method": "tools/call",
                                      "params": {"name": name, "arguments": args}}})
            if result.get("data", {}).get("mode") == "published":
                digest += "-publication"
            trace.append({"direction": "response", "sourceSha256": digest,
                          "payload": {"id": identifier, "result": {"structuredContent": result}}})
        trace.extend([
            {"direction": "request", "sourceSha256": digest, "payload": {"id": -1, "method": "tools/list", "params": {}}},
            {"direction": "response", "sourceSha256": digest, "payload": {"id": -1, "result": {"tools": [
                {"name": name} for name in ["unica.view", "unica.apply", "unica.check"]]}}},
        ])
        for target in [OWNER + ".Template.F06Template.Area.A.Body",
                       OWNER + ".Template.F06Template.Area.A.Parameter",
                       OWNER + ".Template.F06Template.Area.B"]:
            exchange("unica.view", {"at": target}, {"ok": True})
        for number, (operation, target) in enumerate([("template.add", OWNER), ("mxl.set", TEMPLATE)]):
            exchange("unica.view", {"at": target}, {"ok": True, "data": {"can": [{
                "op": operation, "implemented": True, "contract": {"argsSchema": {"type": "object"}}}]}})
            exchange("unica.apply", {"at": target, "ops": [{"op": operation, "args": {}}]},
                     {"ok": True, "data": {"mode": "preview", "effects": 1, "executionToken": str(number)}})
            exchange("unica.apply", {"executionToken": str(number)},
                     {"ok": True, "data": {"mode": "published", "effects": 1}})
        exchange("unica.view", {"at": TEMPLATE + ".Area.Header.Body"}, {"ok": True})
        exchange("unica.check", {"at": TEMPLATE}, {"ok": True, "data": {"status": "passed"}})
        audit(trace)
        def request_index(predicate):
            return next(i for i, r in enumerate(trace) if r["direction"] == "request"
                        and r["payload"]["method"] == "tools/call" and predicate(r["payload"]["params"]))
        preview = request_index(lambda p: bool(p["arguments"].get("ops")))
        publication = request_index(lambda p: "executionToken" in p["arguments"])
        preparatory = request_index(lambda p: p["arguments"].get("at") == OWNER + ".Template.F06Template.Area.A.Parameter")
        foreign = copy.deepcopy(trace)
        foreign[preview]["payload"]["params"]["arguments"]["at"] = "cf:Report.Other"
        with self.assertRaisesRegex(ValueError, "outside"):
            audit(foreign)
        unobserved = copy.deepcopy(trace)
        unobserved[publication]["payload"]["params"]["arguments"]["executionToken"] = "never-observed"
        with self.assertRaisesRegex(ValueError, "observed preview"):
            audit(unobserved)
        writing = copy.deepcopy(trace)
        writing[preview + 1]["sourceSha256"] = "unexpected-write"
        with self.assertRaisesRegex(ValueError, "preview changed"):
            audit(writing)
        missing_read = copy.deepcopy(trace)
        missing_read[preparatory + 1]["payload"]["result"]["structuredContent"]["ok"] = False
        with self.assertRaisesRegex(ValueError, "existing layout"):
            audit(missing_read)
        missing_creation = copy.deepcopy(trace)
        del missing_creation[preview:publication + 2]
        with self.assertRaisesRegex(ValueError, "all requested template operations"):
            audit(missing_creation)

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
