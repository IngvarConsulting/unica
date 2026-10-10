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

    def test_recorder_per_file_snapshot_binds_source_map_owner_and_new_templates(self):
        from tests.agent_evaluation.mcp_recorder import source_observation
        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw)
            source = workspace / "src"
            root = source / "cf"
            root.mkdir(parents=True)
            project = workspace / "v8project.yaml"
            project.write_text("source-set:\n  - name: cf\n    path: src/cf\n")
            owner = root / "Reports/Report.xml"
            owner.parent.mkdir()
            owner.write_text("owner-before")
            before = source_observation(source, workspace, {"cf": "src/cf"})
            self.assertEqual(before["sourceSetRoots"], {"cf": "src/cf"})
            self.assertEqual(before["sourceFilesSha256"], {
                "src/cf/Reports/Report.xml": hashlib.sha256(b"owner-before").hexdigest(),
                "v8project.yaml": hashlib.sha256(project.read_bytes()).hexdigest()})
            body = root / "Reports/Report/Templates/New/Ext/Template.xml"
            body.parent.mkdir(parents=True)
            body.write_text("new-body")
            owner.write_text("registered")
            after = source_observation(source, workspace, {"cf": "src/cf"})
            self.assertEqual(set(after["sourceFilesSha256"]) - set(before["sourceFilesSha256"]),
                             {"src/cf/Reports/Report/Templates/New/Ext/Template.xml"})
            self.assertNotEqual(before["sourceFilesSha256"]["src/cf/Reports/Report.xml"],
                                after["sourceFilesSha256"]["src/cf/Reports/Report.xml"])
            project.write_text("source-set:\n  - name: cf\n    path: src\n")
            remapped = source_observation(source, workspace, {"cf": "src/cf"})
            self.assertNotEqual(remapped["sourceFilesSha256"]["v8project.yaml"],
                                after["sourceFilesSha256"]["v8project.yaml"])
            self.assertEqual(remapped["sourceSetRoots"], after["sourceSetRoots"],
                             "the fixed fixture topology is separate from observed project bytes")

    def test_independent_template_publication_keeps_plan_valid_but_protected_changes_refuse(self):
        import copy
        from tests.agent_evaluation.mcp_recorder import tree_digest
        first = DCS_TEMPLATE
        second = first.rsplit(".Template.", 1)[0] + ".Template.Peer"
        root = "src/cf/Reports/F05Report"
        first_body = root + "/Templates/F05Schema/Ext/Template.xml"
        second_body = root + "/Templates/Peer/Ext/Template.xml"
        files = {path: hashlib.sha256(path.encode()).hexdigest() for path in [
            "v8project.yaml", "src/cf/Configuration.xml", root + ".xml",
            root + "/Templates/F05Schema.xml", first_body,
            root + "/Templates/Peer.xml", second_body]}
        trace = []
        def exchange(name, args, result, write=None):
            identifier = len(trace)
            def observation(direction, payload):
                return {"direction": direction, "payload": payload,
                        "sourceSha256": tree_digest(files), "sourceFilesSha256": dict(files),
                        "sourceSetRoots": {"cf": "src/cf"}}
            method = "tools/list" if name is None else "tools/call"
            params = {} if name is None else {"name": name, "arguments": args}
            trace.append(observation("request", {"id": identifier, "method": method, "params": params}))
            if write:
                files[write] = hashlib.sha256((write + "-published").encode()).hexdigest()
            trace.append(observation("response", {"id": identifier, "result": (
                result if name is None else {"structuredContent": result})}))
        exchange(None, {}, {"tools": [{"name": name} for name in ["unica.view", "unica.apply", "unica.check"]]})
        for template in [first, second]:
            exchange("unica.view", {"at": template}, {"ok": True, "data": {"can": [{
                "op": "field.add", "implemented": True, "contract": {"argsSchema": {"type": "object"}}}]}})
        for token, template in [("first", first), ("peer", second)]:
            exchange("unica.apply", {"at": template, "ops": [{"op": "field.add", "args": {"items": [{"dataPath": "Added"}]}}]},
                     {"ok": True, "data": {"mode": "preview", "effects": 1, "executionToken": token}})
        exchange("unica.apply", {"executionToken": "peer"},
                 {"ok": True, "data": {"mode": "published", "effects": 1}}, second_body)
        publication = len(trace)
        exchange("unica.apply", {"executionToken": "first"},
                 {"ok": True, "data": {"mode": "published", "effects": 1}}, first_body)
        for template in [first, second]:
            exchange("unica.view", {"at": template}, {"ok": True})
            exchange("unica.check", {"at": template}, {"ok": True, "data": {"status": "passed"}})
        def audit(records):
            audit_mcp(records, owner=first.rsplit(".Template.", 1)[0],
                      extra_templates=(second,), required_operations={"field.add"})
        audit(trace)
        for path in [first_body, root + "/Templates/F05Schema.xml", root + ".xml", "src/cf/Configuration.xml",
                     "v8project.yaml", root + "/Templates/F05Schema/Ext/NewInput.xml"]:
            altered = copy.deepcopy(trace)
            altered[publication]["sourceFilesSha256"][path] = hashlib.sha256(b"concurrent input").hexdigest()
            with self.subTest(protected=path), self.assertRaisesRegex(ValueError, "protected plan source"):
                audit(altered)
        outside = copy.deepcopy(trace)
        outside[publication + 1]["sourceFilesSha256"][second_body] = hashlib.sha256(b"foreign write").hexdigest()
        with self.assertRaisesRegex(ValueError, "outside its planned XML"):
            audit(outside)
        owner_write = copy.deepcopy(trace)
        owner_write[publication + 1]["sourceFilesSha256"][root + ".xml"] = hashlib.sha256(b"unexpected registration").hexdigest()
        with self.assertRaisesRegex(ValueError, "outside its planned XML"):
            audit(owner_write)
        remapped = copy.deepcopy(trace)
        remapped[publication]["sourceSetRoots"]["cf"] = "src/other"
        with self.assertRaisesRegex(ValueError, "source map evidence"):
            audit(remapped)
        missing = copy.deepcopy(trace)
        del missing[publication]["sourceFilesSha256"]
        with self.assertRaisesRegex(ValueError, "incomplete per-file"):
            audit(missing)
        preview_index = next(i for i, record in enumerate(trace) if record["direction"] == "response"
                             and ((record["payload"].get("result", {}).get("structuredContent") or {}).get("data") or {}).get("executionToken") == "first")
        for refused in [False, True]:
            writing = copy.deepcopy(trace)
            writing[preview_index]["sourceFilesSha256"]["v8project.yaml"] = hashlib.sha256(b"forbidden preview write").hexdigest()
            if refused:
                writing[preview_index]["payload"]["result"]["structuredContent"]["ok"] = False
            with self.assertRaisesRegex(ValueError, "refused apply changed" if refused else "preview changed"):
                audit(writing)
        # First executes asynchronously; its peer commits while it is pending.
        async_trace = copy.deepcopy(trace)
        first_request, first_response = async_trace[publication:publication + 2]
        del async_trace[publication:publication + 2]
        peer_index = next(i for i, record in enumerate(async_trace) if record["direction"] == "request"
                          and record["payload"].get("params", {}).get("arguments", {}).get("executionToken") == "peer")
        queued_response = copy.deepcopy(first_response)
        queued_response["sourceFilesSha256"] = dict(async_trace[peer_index]["sourceFilesSha256"])
        queued_response["sourceSha256"] = async_trace[peer_index]["sourceSha256"]
        queued_response["payload"]["result"]["structuredContent"] = {"ok": True, "data": {"task": {"taskId": "first-task", "status": "queued"}}}
        first_request["sourceFilesSha256"] = dict(async_trace[peer_index]["sourceFilesSha256"])
        first_request["sourceSha256"] = async_trace[peer_index]["sourceSha256"]
        async_trace[peer_index:peer_index] = [first_request, queued_response]
        terminal_index = peer_index + 4
        terminal_request = copy.deepcopy(first_request)
        terminal_request["payload"] = {"id": 999, "method": "tools/call", "params": {"name": "unica.task.result", "arguments": {"taskId": "first-task"}}}
        terminal_request["sourceFilesSha256"] = dict(async_trace[terminal_index - 1]["sourceFilesSha256"])
        terminal_request["sourceSha256"] = async_trace[terminal_index - 1]["sourceSha256"]
        first_response["payload"]["id"] = 999
        async_trace[terminal_index:terminal_index] = [terminal_request, first_response]
        async_trace[1]["payload"]["result"]["tools"].append({"name": "unica.task.result"})
        audit(async_trace)
        unknown_write = copy.deepcopy(async_trace)
        unknown_write[terminal_index + 1]["sourceFilesSha256"][root + "/Templates/Unobserved/Ext/Template.xml"] = hashlib.sha256(b"unobserved").hexdigest()
        with self.assertRaisesRegex(ValueError, "outside its planned XML"):
            audit(unknown_write)
        overlap = copy.deepcopy(async_trace)
        # A registered peer publication may not change an owner's captured bytes.
        overlap[peer_index + 3]["sourceFilesSha256"][root + ".xml"] = hashlib.sha256(b"owner changed").hexdigest()
        with self.assertRaisesRegex(ValueError, "outside its planned XML|protected plan source"):
            audit(overlap)
        creation_overlap = copy.deepcopy(async_trace)
        logical_owner = first.rsplit(".Template.", 1)[0]
        peer_wrapper = root + "/Templates/Peer.xml"
        # A valid peer creation may write registration; it still invalidates
        # the first plan's captured owner while the first Task is pending.
        for i, record in enumerate(creation_overlap):
            files_at_call = record["sourceFilesSha256"]
            if i < peer_index + 3:
                files_at_call.pop(peer_wrapper, None)
                files_at_call.pop(second_body, None)
            else:
                files_at_call[peer_wrapper] = hashlib.sha256(b"created wrapper").hexdigest()
                files_at_call[root + ".xml"] = hashlib.sha256(b"registered peer").hexdigest()
            record["sourceSha256"] = tree_digest(files_at_call)
        creation_overlap[4]["payload"]["params"]["arguments"]["at"] = logical_owner
        creation_overlap[5]["payload"]["result"]["structuredContent"]["data"]["can"][0]["op"] = "template.add"
        peer_preview = next(record for record in creation_overlap if record["direction"] == "request"
                            and record["payload"].get("params", {}).get("arguments", {}).get("at") == second)
        peer_preview["payload"]["params"]["arguments"] = {"at": logical_owner, "ops": [
            {"op": "template.add", "args": {"items": [{"name": "Peer"}]}}]}
        with self.assertRaisesRegex(ValueError, "protected plan source files during execution"):
            audit(creation_overlap)
        legacy = copy.deepcopy(trace)
        for record in legacy:
            record.pop("sourceFilesSha256")
            record.pop("sourceSetRoots")
        with self.assertRaisesRegex(ValueError, "observed preview"):
            audit(legacy)

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
            exchange("unica.apply", {"at": target, "ops": [{"op": operation, "args": (
                {"items": [{"name": "Agent1311"}]} if operation == "template.add" else {})}]},
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
        foreign_name = copy.deepcopy(trace)
        foreign_name[preview]["payload"]["params"]["arguments"]["ops"][0]["args"]["items"][0]["name"] = "Other"
        with self.assertRaisesRegex(ValueError, "creation names"):
            audit(foreign_name)
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
