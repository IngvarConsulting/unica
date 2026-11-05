"""Deterministic XDTO evidence controls; no product process or Codex execution."""
from __future__ import annotations

import copy
import json
import shutil
import tempfile
import unittest
from pathlib import Path

from tests.agent_evaluation import xdto_driver as driver
from tests.agent_evaluation.mcp_recorder import tree_digest
from tests.ci import xdto_contract_evidence as evidence


class XdtoEvidenceTests(unittest.TestCase):
    def test_source_scenarios_use_the_exact_escaped_property_address(self):
        for name in ["publication", "agent"]:
            with self.subTest(scenario=name):
                path = evidence.REPO / f"tests/fixtures/acceptance/scenarios/S-1721-xdto-{name}.json"
                wire = json.loads(path.read_text())["wire"]
                reads = [step for step in wire if step["tool"] == "unica.view"
                         and "Agent1721" in step["args"].get("at", "")
                         and "СсылкаНаОбъект" in step["args"].get("at", "")]
                self.assertEqual(len(reads), 1)
                self.assertEqual(reads[0]["args"]["at"], evidence.DOTTED)
                self.assertIn({"pointer": "/data/at", "eq": evidence.DOTTED}, reads[0]["assertions"])
                self.assertEqual(reads[0]["args"]["at"].count("\\"), 1)

    def workspace(self, root):
        workspace = root / "workspace"
        shutil.copytree(evidence.FIXTURE, workspace)
        before = evidence.source_hashes(workspace)
        (workspace / evidence.XML).write_bytes(evidence.expected_package())
        return workspace, before

    def test_physical_images_expose_bound_full_xml_and_preserve_neighbors(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace, before = self.workspace(Path(raw))
            observed = json.loads(json.dumps(evidence.verify_xdto_images(workspace, before)))
            self.assertEqual(observed["sourceFilesBeforeSha256"], before)
            self.assertEqual(observed["namespace"], evidence.URI)
            self.assertEqual(observed["ownerName"], "EnterpriseData_1_17_3")
            self.assertEqual(observed["uuid"], "00000000-0000-0000-0000-000000000002")
            self.assertIs(observed["exactExpectedPostimage"], True)
            for path, images in observed["images"].items():
                for phase, source in [("before", evidence.FIXTURE), ("after", workspace)]:
                    self.assertEqual(images[phase]["xmlUtf8"].encode(), (source / path).read_bytes())
                    self.assertEqual(images[phase]["sha256"], evidence.sha((source / path).read_bytes()))
                self.assertTrue(images["bomPreserved"])
                self.assertTrue(images["crlfPreserved"])
                self.assertEqual(images["byteEqual"], path != evidence.XML)

    def test_physical_verifier_rejects_type_namespace_identity_format_and_foreign_changes(self):
        mutations = [
            (evidence.XML, b'type="tns:Agent1721Value"', b'type="xs:string"'),
            (evidence.XML, b'xmlns="http://v8.1c.ru/8.1/xdto"', b'xmlns="urn:foreign"'),
            (evidence.XML, b'lowerBound="0"', b'lowerBound="1"'),
            (evidence.XML, b'name="Agent1721.A"', b'name="Agent1721B"'),
            (evidence.XML, b'\xef\xbb\xbf', b''),
            (evidence.XML, b'\r\n', b'\n'),
            (evidence.DESCRIPTOR, b'00000000-0000-0000-0000-000000000002', b'00000000-0000-0000-0000-000000000004'),
            (evidence.DESCRIPTOR, b'<Name>EnterpriseData_1_17_3</Name>', b'<Name>Other</Name>'),
            (evidence.DESCRIPTOR, b'version="2.20"', b'version="2.21"'),
            ('src/XDTOPackages/ReadonlyNeighbor/Ext/Package.bin', b'</package>', b'<!-- changed --></package>'),
            ('src/Configuration.xml', b'<XDTOPackage>ReadonlyNeighbor</XDTOPackage>', b''),
        ]
        for path, old, new in mutations:
            with self.subTest(path=path, old=old), tempfile.TemporaryDirectory() as raw:
                workspace, before = self.workspace(Path(raw))
                file = workspace / path
                original = file.read_bytes()
                self.assertIn(old, original)
                file.write_bytes(original.replace(old, new))
                with self.assertRaises(ValueError):
                    evidence.verify_xdto_images(workspace, before)
        for mutation in ["create", "remove", "forged preimage"]:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as raw:
                workspace, before = self.workspace(Path(raw))
                if mutation == "create":
                    (workspace / "src/foreign.xml").write_text("<foreign/>")
                elif mutation == "remove":
                    (workspace / "src/XDTOPackages/ReadonlyNeighbor.xml").unlink()
                else:
                    before[evidence.XML] = evidence.sha((workspace / evidence.XML).read_bytes())
                with self.assertRaises(ValueError):
                    evidence.verify_xdto_images(workspace, before)

    def records(self):
        roots = {"main": "src"}
        before = evidence.source_hashes(evidence.FIXTURE)
        before["v8project.yaml"] = evidence.sha((evidence.FIXTURE / "v8project.yaml").read_bytes())
        intermediate = {**before, evidence.XML: evidence.sha(evidence.expected_package("types"))}
        after = {**before, evidence.XML: evidence.sha(evidence.expected_package())}
        records = []
        def pair(method, params, result, old, new):
            request_id = len(records) + 1
            for direction, payload, files in [
                ("request", {"id": request_id, "method": method, "params": params}, old),
                ("response", {"id": request_id, "result": result}, new),
            ]:
                records.append({"direction": direction, "payload": payload,
                                "sourceSha256": tree_digest({p.removeprefix("src/"): d for p, d in files.items() if p.startswith("src/")}),
                                "sourceFilesSha256": files, "sourceSetRoots": roots})
        pair("tools/list", {}, {"tools": [{"name": tool} for tool in ["unica.view", "unica.apply", "unica.check"]]}, before, before)
        def call(tool, args, result, old, new):
            pair("tools/call", {"name": tool, "arguments": args},
                 {"structuredContent": result, "isError": result.get("ok") is False}, old, new)
        call("unica.view", {}, {"ok": True, "data": {}}, before, before)
        requests = list(driver.REQUESTS.items())
        def closed_object(properties, required=()):
            return {"type": "object", "additionalProperties": False,
                    "properties": properties, "required": list(required)}
        name = {"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_.-]*$"}
        qname = {"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_.-]*:[A-Za-z_][A-Za-z0-9_.-]*$"}
        payloads = {
            "valueType.add": closed_object({"name": name, "base": qname}, ["name", "base"]),
            "objectType.add": closed_object({"name": name}, ["name"]),
            "property.add": closed_object({"property": closed_object({"name": name, "type": qname,
                "lowerBound": {"type": "integer", "enum": [0, 1]}}, ["name", "type"])}, ["property"]),
        }
        for index, (selected, old, new) in enumerate([(requests[:2], before, intermediate), (requests[2:], intermediate, after)]):
            operations = []
            for (target, operation), arguments in selected:
                operation_at = target + ".Can." + operation
                properties = {"at": {"type": "string", "minLength": 1}}
                if operation in payloads:
                    properties["values"] = payloads[operation]
                schema = closed_object(properties, ["values"] if operation in payloads else [])
                call("unica.view", {"at": operation_at}, {"ok": True, "at": operation_at, "data": {
                    "at": operation_at, "kind": "Can", "props": {"op": operation,
                    "contract": {"argsSchema": copy.deepcopy(schema)}}}}, old, old)
                operations.append({"op": operation, "args": {"at": target, **copy.deepcopy(arguments)}})
            call("unica.apply", {"at": evidence.PACKAGE, "ops": operations}, {"ok": True, "data": {
                "mode": "preview", "effects": 1, "executionToken": f"token-{index}", "planHash": f"plan-{index}"}}, old, old)
            call("unica.apply", {"executionToken": f"token-{index}"}, {"ok": True, "data": {
                "mode": "published", "effects": 1, "planHash": f"plan-{index}"}}, old, new)
        for target in [evidence.PACKAGE, evidence.OBJECT, evidence.VALUE, evidence.FIELD, evidence.DOTTED]:
            call("unica.view", {"at": target}, {"ok": True, "at": target, "data": {"at": target}}, after, after)
        for target in [evidence.OLD_TYPE, evidence.OLD_PROPERTY]:
            call("unica.view", {"at": target}, {"ok": False, "at": target, "diagnostics": [{"code": "not_found"}]}, after, after)
        call("unica.check", {"at": evidence.PACKAGE}, {"ok": True, "at": evidence.PACKAGE,
             "data": {"status": "passed", "validators": ["xdto"]}}, after, after)
        return records

    def test_audit_accepts_two_saved_publications_with_contextual_contracts_and_readback(self):
        allowed, calls = driver.audit(self.records())
        self.assertEqual(allowed, {"unica.view", "unica.apply", "unica.check"})
        self.assertTrue(calls)
        records = self.records()
        preview = next(r for r in records if r["direction"] == "request" and r["payload"].get("method") == "tools/call"
                       and any(op.get("op") == "property.add" for op in r["payload"]["params"]["arguments"].get("ops", [])))
        preview["payload"]["params"]["arguments"]["ops"][0]["args"]["values"]["property"]["lowerBound"] = 0.0
        driver.audit(records)

    def test_audit_accepts_final_package_check_with_published_limit(self):
        for optional in [{"limit": 50}, {"limit": 1}, {"limit": None}, {"cursor": ""}, {"cursor": None},
                         {"limit": None, "cursor": ""}, {"limit": 50, "cursor": None}]:
            with self.subTest(optional=optional):
                records = self.records()
                request = next(r for r in records if r["direction"] == "request"
                               and r["payload"].get("method") == "tools/call"
                               and r["payload"]["params"]["name"] == "unica.check")
                request["payload"]["params"]["arguments"].update(optional)
                response = next(r for r in records if r["direction"] == "response"
                                and r["payload"]["id"] == request["payload"]["id"])
                response["payload"]["result"]["structuredContent"] = {
                    "ok": True, "at": evidence.PACKAGE, "data": {
                        "at": evidence.PACKAGE, "kind": "XDTOPackage", "status": "passed",
                        "validators": ["xdto"], "diagnosticCount": 0, "diagnostics": []},
                    "page": {"stoppedBy": "complete"},
                }
                driver.audit(records)

    def test_audit_rejects_wrong_check_target_and_illegal_check_envelopes(self):
        for arguments in [
            {"at": evidence.NESTED, "limit": 50},
            {"at": evidence.PACKAGE, "limit": 0},
            {"at": evidence.PACKAGE, "limit": 51},
            {"at": evidence.PACKAGE, "limit": False},
            {"at": evidence.PACKAGE, "limit": "50"},
            {"at": evidence.PACKAGE, "limit": 50, "unknown": True},
            {"at": evidence.PACKAGE, "cursor": False},
            {"at": evidence.PACKAGE, "cursor": 0},
            {"at": evidence.PACKAGE, "cursor": []},
            {"at": evidence.PACKAGE, "cursor": "resume-token"},
        ]:
            with self.subTest(arguments=arguments):
                records = self.records()
                request = next(r for r in records if r["direction"] == "request"
                               and r["payload"].get("method") == "tools/call"
                               and r["payload"]["params"]["name"] == "unica.check")
                request["payload"]["params"]["arguments"] = arguments
                with self.assertRaisesRegex(ValueError, "final XDTO package check"):
                    driver.audit(records)

    def test_audit_rejects_unusable_or_inapplicable_published_can_schemas(self):
        for mutation in ["empty closed schema", "missing values", "missing field", "wrong field type", "invalid schema"]:
            with self.subTest(mutation=mutation):
                records = self.records()
                request = next(r for r in records if r["direction"] == "request"
                               and r["payload"].get("method") == "tools/call"
                               and r["payload"]["params"]["arguments"].get("at") == evidence.PACKAGE + ".Can.valueType.add")
                response = next(r for r in records if r["direction"] == "response" and r["payload"]["id"] == request["payload"]["id"])
                contract = response["payload"]["result"]["structuredContent"]["data"]["props"]["contract"]
                schema = contract["argsSchema"]
                if mutation == "empty closed schema":
                    contract["argsSchema"] = {"type": "object", "additionalProperties": False}
                elif mutation == "missing values":
                    del schema["properties"]["values"]
                elif mutation == "missing field":
                    del schema["properties"]["values"]["properties"]["base"]
                elif mutation == "wrong field type":
                    schema["properties"]["values"]["properties"]["base"] = {"type": "integer"}
                else:
                    schema["properties"]["values"] = {"type": "not-a-json-schema-type"}
                with self.assertRaisesRegex(ValueError, "schema|Schema"):
                    driver.audit(records)

    def test_audit_rejects_transient_or_unbound_recorder_images(self):
        for mutation in ["transient foreign file", "transient immutable hash", "transient package", "forged initial image", "forged final image", "source digest"]:
            with self.subTest(mutation=mutation):
                records = self.records()
                request = next(r for r in records if r["direction"] == "request"
                               and r["payload"].get("method") == "tools/call"
                               and r["payload"]["params"]["arguments"].get("at") == evidence.PACKAGE + ".Can.valueType.add")
                response = next(r for r in records if r["direction"] == "response" and r["payload"]["id"] == request["payload"]["id"])
                frames = [request, response]
                if mutation == "forged initial image":
                    frames = records[:2]
                elif mutation == "forged final image":
                    frames = records[-2:]
                for frame in frames:
                    if mutation == "source digest":
                        frame["sourceSha256"] = "0" * 64
                        continue
                    changed = dict(frame["sourceFilesSha256"])
                    if mutation == "transient foreign file":
                        changed["src/foreign.xml"] = "0" * 64
                    elif mutation == "transient immutable hash":
                        changed["src/XDTOPackages/ReadonlyNeighbor.xml"] = "0" * 64
                    else:
                        changed[evidence.XML] = "0" * 64
                    frame["sourceFilesSha256"] = changed
                    frame["sourceSha256"] = tree_digest({p.removeprefix("src/"): d for p, d in changed.items() if p.startswith("src/")})
                with self.assertRaisesRegex(ValueError, "recorder"):
                    driver.audit(records)

    def test_final_evidence_binds_raw_manifest_and_independent_physical_images(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace, before = self.workspace(Path(raw))
            observations = evidence.verify_xdto_images(workspace, before)
            records = self.records()
            manifest = {"sourceAfter": copy.deepcopy(observations["sourceFilesSha256"])}
            driver.bind_final_evidence(records, manifest, observations)
            for mutation in ["coherent raw final", "manifest final", "physical final", "project binding"]:
                with self.subTest(mutation=mutation):
                    changed_records = copy.deepcopy(records)
                    changed_manifest = copy.deepcopy(manifest)
                    changed_observations = copy.deepcopy(observations)
                    foreign = evidence.sha(b"<different-final-package/>")
                    if mutation == "coherent raw final":
                        for frame in changed_records:
                            files = frame["sourceFilesSha256"]
                            if files[evidence.XML] == manifest["sourceAfter"][evidence.XML]:
                                frame["sourceFilesSha256"] = {**files, evidence.XML: foreign}
                                frame["sourceSha256"] = tree_digest({p.removeprefix("src/"): d for p, d
                                    in frame["sourceFilesSha256"].items() if p.startswith("src/")})
                        # The transcript is internally coherent. Only its binding
                        # to the later, correct physical image detects this gap.
                        driver.audit(changed_records)
                    elif mutation == "manifest final":
                        changed_manifest["sourceAfter"][evidence.XML] = foreign
                    elif mutation == "physical final":
                        changed_observations["sourceFilesSha256"][evidence.XML] = foreign
                    else:
                        changed_observations["projectSHA256"] = foreign
                    with self.assertRaisesRegex(ValueError, "disconnected"):
                        driver.bind_final_evidence(changed_records, changed_manifest, changed_observations)

    def task_records(self):
        records = self.records()
        request = next(r for r in records if r["direction"] == "request"
                       and r["payload"].get("method") == "tools/call"
                       and "executionToken" in r["payload"]["params"]["arguments"])
        response = next(r for r in records if r["direction"] == "response" and r["payload"]["id"] == request["payload"]["id"])
        terminal = copy.deepcopy(response["payload"]["result"])
        after = copy.deepcopy(response)
        response.update({key: copy.deepcopy(request[key]) for key in ["sourceSha256", "sourceFilesSha256", "sourceSetRoots"]})
        response["payload"]["result"] = {"structuredContent": {"ok": True, "data": {
            "task": {"taskId": "publication-task", "status": "queued"}}}, "isError": False}
        next_id = max(r["payload"].get("id", 0) for r in records) + 1
        extra = []
        for offset, (tool, result) in enumerate([
            ("unica.task.get", {"structuredContent": {"ok": True, "data": {
                "task": {"taskId": "publication-task", "status": "working"}}}, "isError": False}),
            ("unica.task.get", {"structuredContent": {"ok": True, "data": {
                "task": {"taskId": "publication-task", "status": "completed"}}}, "isError": False}),
            ("unica.task.result", terminal),
        ]):
            frame = {key: copy.deepcopy(after[key]) for key in ["sourceSha256", "sourceFilesSha256", "sourceSetRoots"]}
            extra.extend([{**copy.deepcopy(frame), "direction": "request", "payload": {"id": next_id + offset,
                "method": "tools/call", "params": {"name": tool, "arguments": {"taskId": "publication-task"}}}},
                {**copy.deepcopy(frame), "direction": "response", "payload": {"id": next_id + offset, "result": result}}])
        at = records.index(response) + 1
        records[at:at] = extra
        inventory = next(r for r in records if r["direction"] == "response" and "tools" in r["payload"].get("result", {}))
        inventory["payload"]["result"]["tools"].extend([{"name": tool} for tool in ["unica.task.get", "unica.task.result"]])
        return records

    def test_audit_accepts_a_task_publication_before_its_terminal_poll(self):
        driver.audit(self.task_records())

    def test_audit_rejects_mcp_error_on_task_origin_or_terminal_result(self):
        for marker in ["executionToken", "taskId"]:
            with self.subTest(marker=marker):
                records = self.task_records()
                request = next(r for r in records if r["direction"] == "request"
                               and r["payload"].get("method") == "tools/call"
                               and marker in r["payload"]["params"]["arguments"]
                               and (marker == "executionToken" or r["payload"]["params"]["name"] == "unica.task.result"))
                response = next(r for r in records if r["direction"] == "response" and r["payload"]["id"] == request["payload"]["id"])
                response["payload"]["result"]["isError"] = True
                with self.assertRaisesRegex(ValueError, "MCP error"):
                    driver.audit(records)

    def test_audit_rejects_contract_token_scope_payload_source_and_verdict_mutations(self):
        for mutation in ["missing Can", "token swap", "preview writes", "foreign publication", "inner at",
                         "flat add", "remove values", "string numeric", "boolean numeric", "fraction",
                         "collection outer", "missing operation", "wrong plan hash", "missing readback", "wrong removed address", "missing validator"]:
            with self.subTest(mutation=mutation):
                records = copy.deepcopy(self.records())
                requests = [r for r in records if r["direction"] == "request" and r["payload"].get("method") == "tools/call"]
                previews = [r for r in requests if "ops" in r["payload"]["params"]["arguments"]]
                preview = previews[0]
                publication = next(r for r in requests if "executionToken" in r["payload"]["params"]["arguments"])
                def response(request):
                    return next(r for r in records if r["direction"] == "response" and r["payload"]["id"] == request["payload"]["id"])
                if mutation == "missing Can":
                    request = next(r for r in requests if r["payload"]["params"]["arguments"].get("at") == evidence.PACKAGE + ".Can.valueType.add")
                    response(request)["payload"]["result"]["structuredContent"]["data"]["props"]["implemented"] = True
                elif mutation == "token swap":
                    publication["payload"]["params"]["arguments"]["executionToken"] = "other"
                elif mutation in {"preview writes", "foreign publication"}:
                    changed = response(preview if mutation == "preview writes" else publication)
                    changed["sourceFilesSha256"] = {**changed["sourceFilesSha256"], "src/foreign.xml": "0" * 64}
                elif mutation == "inner at":
                    preview["payload"]["params"]["arguments"]["ops"][0]["args"]["values"]["at"] = False
                elif mutation == "flat add":
                    arguments = preview["payload"]["params"]["arguments"]["ops"][0]["args"]
                    arguments.update(arguments.pop("values"))
                elif mutation == "remove values":
                    operations = previews[1]["payload"]["params"]["arguments"]["ops"]
                    next(op for op in operations if op["op"] == "type.remove")["args"]["values"] = {}
                elif mutation in {"string numeric", "boolean numeric", "fraction"}:
                    previews[1]["payload"]["params"]["arguments"]["ops"][0]["args"]["values"]["property"]["lowerBound"] = {
                        "string numeric": "0", "boolean numeric": False, "fraction": 0.5}[mutation]
                elif mutation == "collection outer":
                    preview["payload"]["params"]["arguments"]["at"] = "main:XDTOPackage"
                elif mutation == "missing operation":
                    previews[1]["payload"]["params"]["arguments"]["ops"].pop()
                elif mutation == "wrong plan hash":
                    response(publication)["payload"]["result"]["structuredContent"]["data"]["planHash"] = "other"
                elif mutation == "missing readback":
                    next(r for r in requests if r["payload"]["params"]["arguments"].get("at") == evidence.DOTTED)["payload"]["params"]["arguments"]["at"] = evidence.NESTED
                elif mutation == "wrong removed address":
                    request = next(r for r in requests if r["payload"]["params"]["arguments"].get("at") == evidence.OLD_PROPERTY)
                    response(request)["payload"]["result"]["structuredContent"]["at"] = evidence.OLD_TYPE
                else:
                    request = next(r for r in requests if r["payload"]["params"]["name"] == "unica.check")
                    response(request)["payload"]["result"]["structuredContent"]["data"] = {"status": "readable", "validators": []}
                with self.assertRaises(ValueError):
                    driver.audit(records)

    def test_audit_rejects_successful_mcp_error_at_each_observation_boundary(self):
        for tool, marker in [("unica.view", ".Can.valueType.add"), ("unica.apply", "ops"),
                             ("unica.apply", "executionToken"), ("unica.view", evidence.DOTTED), ("unica.check", evidence.PACKAGE)]:
            with self.subTest(tool=tool, marker=marker):
                records = self.records()
                request = next(r for r in records if r["direction"] == "request" and r["payload"].get("method") == "tools/call"
                               and r["payload"]["params"]["name"] == tool
                               and (marker in r["payload"]["params"]["arguments"]
                                    or str(r["payload"]["params"]["arguments"].get("at", "")).endswith(marker)))
                response = next(r for r in records if r["direction"] == "response" and r["payload"]["id"] == request["payload"]["id"])
                response["payload"]["result"]["isError"] = True
                with self.assertRaisesRegex(ValueError, "MCP error"):
                    driver.audit(records)
