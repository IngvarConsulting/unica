"""A forbidden evidence path cannot become a passing agent evaluation."""
import unittest
import tempfile
import json
import hashlib
import os
from unittest.mock import patch
from pathlib import Path

from tests.agent_evaluation.dcs_driver import DCS_TEMPLATE, allowed_help_command, audit_cli, audit_mcp, audit_review, terminal_calls


def detailed_can_response(target, operation):
    at = target + ".Can." + operation
    return {"ok": True, "at": at, "data": {"at": at, "kind": "Can", "props": {
        "op": operation, "implemented": True, "contract": {"argsSchema": {"type": "object"}}}}}


def evaluate_mxl_fixture(root, mutation=None):
    import shutil
    from types import SimpleNamespace
    from tests.agent_evaluation import mxl_driver
    from tests.ci.acceptance_controls import load_acceptance_corpus
    scenario = next(scenario for scenario in load_acceptance_corpus()["scenarios"] if scenario["id"] == "S335")
    workspace = root / "workspace"
    shutil.copytree(mxl_driver.REPO / scenario["workspace"], workspace)
    body = workspace / (mxl_driver.PREFIX + "Templates/Agent1311/Ext/Template.xml")
    created = '''<document xmlns="http://v8.1c.ru/8.2/data/spreadsheet" xmlns:v8="http://v8.1c.ru/8.1/data/core">
<columns><size>4</size></columns>
<rowsItem><index>0</index><row>
<c><c><tl><v8:item><v8:content>Товар</v8:content></v8:item></tl></c></c>
<c><c><tl><v8:item><v8:content>Сумма</v8:content></v8:item></tl></c></c>
<c><c><tl><v8:item><v8:content/></v8:item></tl></c></c><c><c/></c>
</row></rowsItem>
<rowsItem><index>1</index><row>
<c><c><tl><v8:item><v8:content>Agent1311</v8:content></v8:item></tl></c></c>
<c><c><tl><v8:item><v8:content>12.5</v8:content></v8:item></tl></c></c>
<c><c/></c><c><c/></c>
</row></rowsItem>
<namedItem><name>Header</name><area><beginRow>0</beginRow><endRow>1</endRow></area></namedItem>
</document>'''

    def completed_agent(server, scenario, proof_root, **kwargs):
        # Simulate only the external agent boundary. The wrapper must observe
        # actual files after it returns; no observation result is mocked.
        before = {path.relative_to(workspace).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                  for path in (workspace / "src").rglob("*") if path.is_file()}
        owner = workspace / "src/cf/Reports/F06Report.xml"
        owner.write_text(owner.read_text().replace("</ChildObjects>", "<Template>Agent1311</Template></ChildObjects>"))
        descriptor = workspace / (mxl_driver.PREFIX + "Templates/Agent1311.xml")
        descriptor.write_text('''<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Template><Properties>
<TemplateType>SpreadsheetDocument</TemplateType></Properties></Template></MetaDataObject>''')
        body.parent.mkdir(parents=True)
        body.write_text(created)
        if mutation:
            mutation(workspace, scenario)
        proof = root / "proof"
        proof.mkdir()
        (proof / "manifest.json").write_text(json.dumps({"sourceBefore": before}))
        return proof

    with patch("tests.agent_evaluation.mxl_driver.evaluate_contract", side_effect=completed_agent):
        return mxl_driver.evaluate(SimpleNamespace(workspace=workspace), scenario, root / "proofs")


class AgentEvaluationAuditTests(unittest.TestCase):
    def test_mxl_evaluation_records_physical_outcome_and_passes_it_to_independent_review(self):
        from types import SimpleNamespace
        from tests.agent_evaluation import mxl_driver
        with tempfile.TemporaryDirectory() as raw:
            proof = evaluate_mxl_fixture(Path(raw))
            observations = proof / "file-observations.json"
            self.assertTrue(observations.exists(), "MXL evaluation must save actual physical observations")
            actual = json.loads(observations.read_text())
            self.assertGreater(len(actual["fileAssertions"]), 10)
            self.assertTrue(actual["preservedTemplatesSha256"])
            self.assertIn("Agent1311", actual["registeredTemplates"])
            self.assertEqual(json.loads((proof / "manifest.json").read_text())["fileObservationsSha256"],
                             hashlib.sha256(observations.read_bytes()).hexdigest())
            for name, content in {"prompt.md": "Create Header", "final.json": "{}", "mcp.jsonl": "",
                                  "verification.json": "[]", "scenario.json": "{}", "agent.jsonl": ""}.items():
                (proof / name).write_text(content)
            captured = []

            def review(command, prompt, environment, output, **kwargs):
                captured.append(prompt)
                (proof / "review.json").write_text(json.dumps({"accepted": True, "reasons": "fixture review"}))
                transcript = proof / "reviewer.jsonl"
                transcript.write_text("")
                return [{"type": "turn.completed"}], transcript, proof / "stderr"

            with patch("tests.agent_evaluation.dcs_driver.codex_command", return_value=[]), \
                 patch("tests.agent_evaluation.dcs_driver.disabled_skills", return_value=[]), \
                 patch("tests.agent_evaluation.dcs_driver.run_cli", side_effect=review), \
                 patch("tests.agent_evaluation.dcs_driver.packager", return_value=SimpleNamespace(
                     sha256=lambda path: hashlib.sha256(path.read_bytes()).hexdigest())):
                mxl_driver.independent_review(proof)
            evidence = json.loads(captured[0].split("\nСвидетельства:\n", 1)[1])
            self.assertEqual(evidence["fileObservations"], actual)
            self.assertIn("file-observations.json", json.loads((proof / "receipt.json").read_text())["files"])

    def test_mxl_physical_outcome_rejects_wrong_cells_width_rows_neighbours_and_old_resources(self):
        from tests.agent_evaluation.mxl_driver import PREFIX

        def change_file(path, old, new):
            def mutate(workspace, scenario):
                file = workspace / path
                file.write_text(file.read_text().replace(old, new))
            return mutate

        body = PREFIX + "Templates/Agent1311/Ext/Template.xml"
        for name, mutation in [
            ("width", change_file(body, "<size>4</size>", "<size>3</size>")),
            ("rows", change_file(body, "<index>1</index>", "<index>2</index>")),
            ("cell", change_file(body, "Товар", "Wrong")),
            ("position", change_file(body, "<c><c><tl>", "<c><i>1</i><c><tl>")),
            ("neighbour", change_file(body, "<v8:content/>", "<v8:content>unexpected</v8:content>")),
            ("area", change_file(body, "<name>Header</name>", "<name>Other</name>")),
            ("area bounds", change_file(body, "<endRow>1</endRow>", "<endRow>2</endRow>")),
            ("descriptor kind", change_file(PREFIX + "Templates/Agent1311.xml", "SpreadsheetDocument", "DataCompositionSchema")),
            ("old resource", change_file(PREFIX + "Templates/F06Template/Ext/Template.xml", "Changed", "Corrupted")),
            ("lost registration", change_file("src/cf/Reports/F06Report.xml", "<Template>F06Template</Template>", "")),
            ("extra registration", change_file("src/cf/Reports/F06Report.xml", "</ChildObjects>", "<Template>Other</Template></ChildObjects>")),
            ("lost resource", lambda workspace, scenario: (workspace / (PREFIX + "Templates/F06Template.xml")).unlink()),
            ("missing assertions", lambda workspace, scenario: [step.pop("fileAssertions", None) for step in scenario["wire"]]),
        ]:
            with self.subTest(physical_mutation=name), tempfile.TemporaryDirectory() as raw:
                with self.assertRaises(ValueError):
                    evaluate_mxl_fixture(Path(raw), mutation)

    def test_detailed_can_authorizes_only_its_exact_successful_preceding_target_and_operation(self):
        import copy
        target = DCS_TEMPLATE + ".DataSet.F05Data"
        operation = "field.add"
        can_at = target + ".Can." + operation
        trace = []
        digest = "before"

        def exchange(name, args, result):
            nonlocal digest
            identifier = len(trace)
            method = "tools/list" if name is None else "tools/call"
            params = {} if name is None else {"name": name, "arguments": args}
            trace.append({"direction": "request", "sourceSha256": digest,
                          "payload": {"id": identifier, "method": method, "params": params}})
            if result.get("data", {}).get("mode") == "published":
                digest = "after"
            trace.append({"direction": "response", "sourceSha256": digest,
                          "payload": {"id": identifier, "result": (
                              result if name is None else {"structuredContent": result})}})

        exchange(None, {}, {"tools": [{"name": name} for name in ["unica.view", "unica.apply", "unica.check"]]})
        exchange("unica.view", {"at": can_at}, detailed_can_response(target, operation))
        exchange("unica.apply", {"at": target, "ops": [{"op": operation, "args": {
            "items": [{"dataPath": "Added"}]}}]},
                 {"ok": True, "data": {"mode": "preview", "effects": 1, "executionToken": "plan"}})
        exchange("unica.apply", {"executionToken": "plan"},
                 {"ok": True, "data": {"mode": "published", "effects": 1}})
        exchange("unica.view", {"at": DCS_TEMPLATE}, {"ok": True})
        exchange("unica.check", {"at": DCS_TEMPLATE}, {"ok": True, "data": {"status": "passed"}})

        def audit(records):
            audit_mcp(records, required_operations={operation})

        audit(trace)
        original = detailed_can_response(target, operation)
        alternatives = {}
        for key, value in [("ok", False), ("at", DCS_TEMPLATE + ".Can." + operation)]:
            response = copy.deepcopy(original)
            response[key] = value
            alternatives[key] = response
        for key, value in [("at", DCS_TEMPLATE + ".Can." + operation), ("kind", "Template")]:
            response = copy.deepcopy(original)
            response["data"][key] = value
            alternatives["data." + key] = response
        for key, value in [("op", "field.remove"), ("implemented", False), ("contract", None)]:
            response = copy.deepcopy(original)
            response["data"]["props"][key] = value
            alternatives["props." + key] = response
        for schema in [None, {}, [], "object", {"type": "string"}]:
            response = copy.deepcopy(original)
            response["data"]["props"]["contract"]["argsSchema"] = schema
            alternatives["schema " + repr(schema)] = response
        alternatives["shortlist"] = {"ok": True, "at": target, "data": {"at": target,
            "kind": "DataSet", "can": [{"op": operation, "implemented": True}]}}
        alternatives["retired inline contract"] = {"ok": True, "at": target, "data": {
            "at": target, "kind": "DataSet", "can": [{"op": operation, "implemented": True,
            "contract": {"argsSchema": {"type": "object"}}}]}}
        for label, response in alternatives.items():
            altered = copy.deepcopy(trace)
            altered[3]["payload"]["result"]["structuredContent"] = response
            if label in {"shortlist", "retired inline contract"}:
                altered[2]["payload"]["params"]["arguments"]["at"] = target
            with self.subTest(response=label), self.assertRaisesRegex(ValueError, "detailed contract"):
                audit(altered)
        for missing_path in [("at",), ("data", "at"), ("data", "kind"), ("data", "props", "op"),
                             ("data", "props", "implemented"), ("data", "props", "contract", "argsSchema")]:
            altered = copy.deepcopy(trace)
            response = altered[3]["payload"]["result"]["structuredContent"]
            for key in missing_path[:-1]:
                response = response[key]
            response.pop(missing_path[-1])
            with self.subTest(missing=missing_path), self.assertRaisesRegex(ValueError, "detailed contract"):
                audit(altered)
        transport_failure = copy.deepcopy(trace)
        transport_failure[3]["payload"]["result"]["isError"] = True
        with self.assertRaisesRegex(ValueError, "detailed contract"):
            audit(transport_failure)
        asynchronous = copy.deepcopy(trace)
        asynchronous[1]["payload"]["result"]["tools"].append({"name": "unica.task.result"})
        asynchronous[3]["payload"]["result"]["structuredContent"] = {
            "ok": True, "data": {"task": {"taskId": "contract-read", "status": "queued"}}}
        completion = [
            {"direction": "request", "sourceSha256": "before", "payload": {"id": 99, "method": "tools/call",
             "params": {"name": "unica.task.result", "arguments": {"taskId": "contract-read"}}}},
            {"direction": "response", "sourceSha256": "before", "payload": {"id": 99, "result": {
                "structuredContent": detailed_can_response(target, operation)}}},
        ]
        asynchronous[4:4] = completion
        audit(asynchronous)
        asynchronous[5]["payload"]["result"]["isError"] = True
        with self.assertRaisesRegex(ValueError, "detailed contract"):
            audit(asynchronous)
        missing = copy.deepcopy(trace)
        del missing[2:4]
        with self.assertRaisesRegex(ValueError, "detailed contract"):
            audit(missing)
        late = copy.deepcopy(trace)
        late[2:6] = late[4:6] + late[2:4]
        with self.assertRaisesRegex(ValueError, "detailed contract"):
            audit(late)
        late_response = copy.deepcopy(trace)
        late_response[3], late_response[4] = late_response[4], late_response[3]
        with self.assertRaisesRegex(ValueError, "detailed contract"):
            audit(late_response)
        for other_target, other_operation in [(DCS_TEMPLATE, operation), (target, "field.remove")]:
            altered = copy.deepcopy(trace)
            altered[2]["payload"]["params"]["arguments"]["at"] = other_target + ".Can." + other_operation
            altered[3]["payload"]["result"]["structuredContent"] = detailed_can_response(other_target, other_operation)
            with self.subTest(target=other_target, operation=other_operation), self.assertRaisesRegex(ValueError, "detailed contract"):
                audit(altered)
        for key, value in [("dataSet", "F05Data"), ("variant", "F05Variant")]:
            altered = copy.deepcopy(trace)
            selected = DCS_TEMPLATE + (".DataSet." if key == "dataSet" else ".Setting.") + value
            altered[2]["payload"]["params"]["arguments"]["at"] = selected + ".Can." + operation
            altered[3]["payload"]["result"]["structuredContent"] = detailed_can_response(selected, operation)
            altered[4]["payload"]["params"]["arguments"]["at"] = DCS_TEMPLATE
            altered[4]["payload"]["params"]["arguments"]["ops"][0]["args"]["values"] = {key: value}
            with self.subTest(retired_selector=key), self.assertRaisesRegex(ValueError, "detailed contract"):
                audit(altered)

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
            exchange("unica.view", {"at": template + ".Can.field.add"}, detailed_can_response(template, "field.add"))
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
        creation_overlap[4]["payload"]["params"]["arguments"]["at"] = logical_owner + ".Can.template.add"
        creation_overlap[5]["payload"]["result"]["structuredContent"] = detailed_can_response(logical_owner, "template.add")
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
            for op, suffix in [("field.add", ".DataSet.F05Data"), ("query.set", ".DataSet.F05Data.Query"),
                               ("structure.patch", ".Setting.F05Variant.Item.F05Group")]:
                target = DCS_TEMPLATE + suffix
                exchange("tools/call", {"name": "unica.view", "arguments": {"at": target + ".Can." + op}},
                         detailed_can_response(target, op))
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
        for number, (operation, target) in enumerate([("template.add", OWNER), ("area.add", TEMPLATE)]):
            exchange("unica.view", {"at": target + ".Can." + operation}, detailed_can_response(target, operation))
            exchange("unica.apply", {"at": target, "ops": [{"op": operation, "args": (
                {"items": [{"name": "Agent1311", "kind": "SpreadsheetDocument"}]} if operation == "template.add" else
                {"items": [{"name": "Header", "columns": 4, "cells": {
                    "R1C1": "Товар", "R1C2": "Сумма", "R2C1": "Agent1311", "R2C2": 12.5}}]})}]},
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
