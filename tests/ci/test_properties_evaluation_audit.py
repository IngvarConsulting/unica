"""Properties agent evidence checks run without Codex or a product process."""
from __future__ import annotations

import copy
import hashlib
import json
import shutil
import tempfile
import types
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
from unittest.mock import patch

from tests.agent_evaluation import dcs_driver, props_driver
from tests.agent_evaluation.mcp_recorder import source_file_hashes, tree_digest


class PropertiesEvidenceTests(unittest.TestCase):
    def source_hashes(self, workspace):
        return {"src/" + path: digest for path, digest in source_file_hashes(workspace / "src").items()}

    def prepared_workspace(self, root):
        workspace = root / "workspace"
        shutil.copytree(props_driver.SOURCE_FIXTURE, workspace)
        before = self.source_hashes(workspace)
        for path, raw in props_driver.expected_descriptor_images().items():
            (workspace / path).write_bytes(raw)
        return workspace, before

    def test_physical_verifier_accepts_exact_requested_postimages(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace, before = self.prepared_workspace(Path(raw))
            observed = props_driver.verify_properties_images(workspace, before)
            self.assertEqual(set(observed["descriptors"]), set(props_driver.PATHS))
            self.assertTrue(observed["descriptors"][props_driver.MODULE]["bom"])
            self.assertTrue(observed["descriptors"][props_driver.SUBSYSTEM]["crlf"])

    def test_physical_export_exposes_bound_full_xml_owner_languages_and_retained_companions(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace, before = self.prepared_workspace(Path(raw))
            observed = json.loads(json.dumps(props_driver.verify_properties_images(workspace, before), ensure_ascii=False))
            self.assertEqual(observed["sourceFilesBeforeSha256"], before)
            self.assertEqual(set(observed["companions"]), {
                "src/Roles/SalesReader/Ext/Rights.xml",
                "src/Subsystems/Администрирование/Ext/CommandInterface.xml"})
            expected = props_driver.expected_descriptor_images()
            for target, path in props_driver.PATHS.items():
                with self.subTest(target=target):
                    descriptor = observed["descriptors"][target]
                    kind, name = target.split(":", 1)[1].split(".", 1)
                    self.assertEqual(descriptor["ownerName"], name)
                    self.assertIs(descriptor["exactExpectedPostimage"], True)
                    self.assertEqual(descriptor["expectedPostimageSHA256"], hashlib.sha256(expected[path]).hexdigest())
                    for phase, image in [("before", (props_driver.SOURCE_FIXTURE / path).read_bytes()),
                                         ("after", (workspace / path).read_bytes())]:
                        evidence = descriptor[phase]
                        self.assertEqual(evidence["xmlUtf8"].encode("utf-8"), image)
                        self.assertEqual(evidence["sha256"], hashlib.sha256(image).hexdigest())
                        self.assertEqual(evidence["bom"], image.startswith(b"\xef\xbb\xbf"))
                        self.assertEqual(evidence["crlf"], b"\r\n" in image)
                        parsed = ET.fromstring(evidence["xmlUtf8"])
                        self.assertEqual(evidence["allLanguages"], list(dict.fromkeys(
                            node.text for node in parsed.iter(props_driver.CORE + "lang"))))
                        owner = parsed.find(props_driver.MD + kind)
                        self.assertEqual(owner.findtext(props_driver.MD + "Properties/" + props_driver.MD + "Name"), name)
                        self.assertEqual(evidence["ownerName"], name)
                        self.assertEqual(evidence["languages"]["Synonym"], ["ru"])
                    self.assertEqual(descriptor["before"]["sha256"], before[path])
                    self.assertEqual(descriptor["after"]["sha256"], observed["sourceFilesSha256"][path])
            subsystem = observed["descriptors"][props_driver.SUBSYSTEM]
            self.assertEqual(subsystem["before"]["languages"]["Explanation"], ["ru"])
            self.assertEqual(subsystem["after"]["languages"]["Explanation"], [])
            for path, companion in observed["companions"].items():
                self.assertIs(companion["byteEqual"], True)
                self.assertEqual(companion["after"]["sha256"], observed["sourceFilesSha256"][path])
                for phase in ["before", "after"]:
                    evidence = companion[phase]
                    self.assertEqual(evidence["xmlUtf8"].encode("utf-8"), (props_driver.SOURCE_FIXTURE / path).read_bytes())
                    self.assertEqual(evidence["sha256"], before[path])
                    ET.fromstring(evidence["xmlUtf8"])

    def test_physical_export_inventories_languages_outside_the_edited_properties(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            fixture = root / "fixture"
            shutil.copytree(props_driver.SOURCE_FIXTURE, fixture)
            module = fixture / props_driver.PATHS[props_driver.MODULE]
            original = module.read_bytes()
            extra = (b'<extra:label xmlns:extra="urn:retained-fixture"><v8:item><v8:lang>en</v8:lang>'
                     b'<v8:content>Retained extension label</v8:content></v8:item></extra:label>')
            module.write_bytes(original.replace(b"</CommonModule>", extra + b"</CommonModule>"))
            with patch.object(props_driver, "SOURCE_FIXTURE", fixture):
                workspace, before = self.prepared_workspace(root)
                observed = props_driver.verify_properties_images(workspace, before)
            for phase in ["before", "after"]:
                evidence = observed["descriptors"][props_driver.MODULE][phase]
                self.assertEqual(evidence["languages"]["Synonym"], ["ru"])
                self.assertEqual(evidence["allLanguages"], ["ru", "en"])
                parsed = ET.fromstring(evidence["xmlUtf8"])
                self.assertEqual(parsed.findtext(".//{urn:retained-fixture}label/" + props_driver.CORE + "item/"
                                                 + props_driver.CORE + "content"), "Retained extension label")

    def test_physical_verifier_rejects_unbound_descriptor_before_images(self):
        for path in props_driver.PATHS.values():
            for mutation in ["missing", "forged"]:
                with self.subTest(path=path, mutation=mutation), tempfile.TemporaryDirectory() as raw:
                    workspace, before = self.prepared_workspace(Path(raw))
                    if mutation == "missing":
                        del before[path]
                    else:
                        before[path] = hashlib.sha256((workspace / path).read_bytes()).hexdigest()
                    with self.assertRaises(ValueError):
                        props_driver.verify_properties_images(workspace, before)

    def test_physical_verifier_rejects_wrong_type_namespace_missing_and_foreign_edits(self):
        module = props_driver.PATHS[props_driver.MODULE]
        role = props_driver.PATHS[props_driver.ROLE]
        changes = {
            "wrong boolean spelling": (module, b">false</ClientOrdinaryApplication>", b'>"false"</ClientOrdinaryApplication>'),
            "foreign descriptor namespace": (role, b"http://v8.1c.ru/8.3/MDClasses", b"urn:foreign:metadata"),
            "foreign property namespace": (role, b"<Comment>Agent1721 role comment</Comment>",
                b'<foreign:Comment xmlns:foreign="urn:foreign">Agent1721 role comment</foreign:Comment>'),
            "missing canonical property": (module, b"<Comment>Agent1721 module</Comment>", b""),
            "duplicate canonical property": (role, b"<Comment>Agent1721 role comment</Comment>",
                b"<Comment>Agent1721 role comment</Comment><Comment>Agent1721 role comment</Comment>"),
            "changed owner identity": (role, b"00000000-0000-0000-0000-000000000051", b"00000000-0000-0000-0000-000000000052"),
            "lost BOM": (module, b"\xef\xbb\xbf", b""),
            "changed line endings": (module, b"\r\n", b"\n"),
            "foreign rights edit": ("src/Roles/SalesReader/Ext/Rights.xml", b"</Rights>", b"<!-- changed --></Rights>"),
            "foreign command interface edit": ("src/Subsystems/Администрирование/Ext/CommandInterface.xml",
                                                b"</CommandInterface>", b"<!-- changed --></CommandInterface>"),
        }
        for label, (path, old, new) in changes.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as raw:
                workspace, before = self.prepared_workspace(Path(raw))
                file = workspace / path
                original = file.read_bytes()
                self.assertIn(old, original)
                file.write_bytes(original.replace(old, new))
                with self.assertRaises(ValueError):
                    props_driver.verify_properties_images(workspace, before)
        for action in ["remove", "create"]:
            with self.subTest(action=action), tempfile.TemporaryDirectory() as raw:
                workspace, before = self.prepared_workspace(Path(raw))
                if action == "remove":
                    (workspace / "src/Roles/SalesReader/Ext/Rights.xml").unlink()
                else:
                    (workspace / "src/foreign.xml").write_text("<foreign/>")
                with self.assertRaises(ValueError):
                    props_driver.verify_properties_images(workspace, before)

    def accepted_records(self):
        roots = {"main": "src"}
        before = {"src/" + name: digest for name, digest in source_file_hashes(props_driver.SOURCE_FIXTURE / "src").items()}
        before["v8project.yaml"] = hashlib.sha256((props_driver.SOURCE_FIXTURE / "v8project.yaml").read_bytes()).hexdigest()
        after = {**before, **{path: hashlib.sha256(raw).hexdigest()
                             for path, raw in props_driver.expected_descriptor_images().items()}}
        records = []
        def pair(method, params, result, old, new):
            request_id = len(records) + 1
            for direction, payload, files in [
                ("request", {"id": request_id, "method": method, "params": params}, old),
                ("response", {"id": request_id, "result": result}, new),
            ]:
                records.append({"direction": direction, "payload": payload,
                    "sourceSha256": tree_digest({p: d for p, d in files.items() if p.startswith("src/")}),
                    "sourceFilesSha256": files, "sourceSetRoots": roots})
        inventory = ["unica.view", "unica.apply", "unica.check"]
        pair("tools/list", {}, {"tools": [{"name": tool} for tool in inventory]}, before, before)
        def call(tool, args, result, old=before, new=before):
            pair("tools/call", {"name": tool, "arguments": args},
                 {"structuredContent": result, "isError": result.get("ok") is False}, old, new)
        call("unica.view", {}, {"ok": True, "data": {}})
        call("unica.view", {"at": "main:Configuration.Can.props.set"}, self.configuration_contract_result())
        for target in props_driver.PATHS:
            at = target + ".Can.props.set"
            call("unica.view", {"at": at}, {"ok": True, "at": at, "data": {"at": at, "kind": "Can", "props": {
                "op": "props.set", "contract": {"argsSchema": {"type": "object", "additionalProperties": False}}}}})
        values_by_target = {
            props_driver.MODULE: props_driver.TARGET_VALUES[props_driver.MODULE],
            props_driver.ROLE: {"synonym": "Agent1721 role", "Comment": "Agent1721 role comment"},
            props_driver.SUBSYSTEM: {"Synonym": "Agent1721 subsystem", "comment": "Agent1721 subsystem comment",
                                     "explanation": "", "IncludeInCommandInterface": False},
        }
        current = before
        for index, (target, values) in enumerate(values_by_target.items()):
            token, plan_hash = f"props-token-{index}", f"sha256:props-{index}"
            call("unica.apply", {"at": target, "ops": [{"op": "props.set", "args": {"values": values}}]},
                 {"ok": True, "data": {"mode": "preview", "effects": 1,
                  "planHash": plan_hash, "executionToken": token}}, current, current)
            next_image = {**current, props_driver.PATHS[target]: after[props_driver.PATHS[target]]}
            call("unica.apply", {"executionToken": token},
                 {"ok": True, "data": {"mode": "published", "effects": 1, "planHash": plan_hash}},
                 current, next_image)
            current = next_image
        for target in props_driver.PATHS:
            call("unica.view", {"at": target}, {"ok": True, "data": {}}, after, after)
            result = ({"ok": False, "data": {"tool": "bsl-analyzer", "stage": "manifest"},
                       "diagnostics": [{"code": "provider_unavailable", "detailCode": "provider_absent"}]} if target == props_driver.MODULE
                      else {"ok": True, "data": {"status": "passed"}})
            call("unica.check", {"at": target}, result, after, after)
        return records

    def configuration_contract_result(self):
        at = "main:Configuration.Can.props.set"
        return {"ok": True, "at": at, "data": {"at": at, "kind": "Can", "props": {
            "op": "props.set", "args": "values", "contract": {"argsSchema": {
                "type": "object", "additionalProperties": False, "required": ["values"],
                "properties": {"values": {
                    "type": "object", "additionalProperties": False, "minProperties": 1,
                    "properties": {"Comment": {"type": "string"}, "Vendor": {"type": "string"}},
                }},
            }}}}}

    def configuration_records(self):
        records = self.accepted_records()
        request = next(record for record in records if record["direction"] == "request"
                       and record["payload"].get("method") == "tools/call"
                       and record["payload"]["params"]["arguments"].get("at") == "main:Configuration.Can.props.set")
        response = next(record for record in records if record["direction"] == "response"
                        and record["payload"]["id"] == request["payload"]["id"])
        return records, request, response

    def test_audit_accepts_configuration_comment_from_its_detailed_contract(self):
        allowed, calls = props_driver.audit(self.accepted_records())
        self.assertEqual(allowed, {"unica.view", "unica.apply", "unica.check"})
        self.assertTrue(calls)

    def test_audit_rejects_configuration_catalog_names_without_the_comment_contract(self):
        for at, data in [
            ("main:Configuration", {"at": "main:Configuration", "kind": "Configuration", "can": [
                {"op": "props.set", "args": "values"}, {"op": "object.create", "args": "values"}]}),
            ("main:Configuration", {"at": "main:Configuration", "kind": "Configuration", "can": [
                {"op": "object.create", "args": "values"}]}),
            ("main:Configuration.Can", {"items": []}),
            ("main:Configuration.Can", {"items": [{"at": "main:Configuration.Can.props.set",
                                                   "props": {"op": "props.set"}}]}),
        ]:
            with self.subTest(at=at, data=data):
                records, request, response = self.configuration_records()
                request["payload"]["params"]["arguments"]["at"] = at
                response["payload"]["result"]["structuredContent"] = {"ok": True, "at": at, "data": data}
                with self.assertRaisesRegex(ValueError, "Configuration inspection"):
                    props_driver.audit(records)

    def test_audit_rejects_unproven_configuration_comment_contracts(self):
        mutations = [
            ("foreign result at", ["at"], props_driver.ROLE),
            ("foreign data at", ["data", "at"], props_driver.ROLE),
            ("foreign kind", ["data", "kind"], "Role"),
            ("foreign op", ["data", "props", "op"], "object.create"),
            ("empty op", ["data", "props", "op"], ""),
            ("whitespace op", ["data", "props", "op"], " props.set "),
            ("non-text op", ["data", "props", "op"], 7),
            ("missing args", ["data", "props", "args"], None),
            ("wrong args", ["data", "props", "args"], "items"),
            ("non-text args", ["data", "props", "args"], {}),
            ("stale implemented", ["data", "props", "implemented"], True),
            ("missing contract", ["data", "props", "contract"], None),
            ("non-object props", ["data", "props"], ["props.set"]),
            ("non-object contract", ["data", "props", "contract"], []),
        ]
        schema = ["data", "props", "contract", "argsSchema"]
        mutations += [
            ("missing schema", schema, None),
            ("non-object schema", schema, []),
            ("wrong schema type", schema + ["type"], "string"),
            ("open schema", schema + ["additionalProperties"], True),
            ("optional values", schema + ["required"], []),
            ("non-list required", schema + ["required"], "values"),
            ("missing values", schema + ["properties", "values"], None),
            ("non-object values", schema + ["properties", "values"], []),
            ("wrong values type", schema + ["properties", "values", "type"], "string"),
            ("open values", schema + ["properties", "values", "additionalProperties"], True),
            ("empty values", schema + ["properties", "values", "minProperties"], 0),
            ("boolean size", schema + ["properties", "values", "minProperties"], True),
            ("missing Comment", schema + ["properties", "values", "properties", "Comment"], None),
            ("wrong Comment type", schema + ["properties", "values", "properties", "Comment", "type"], "boolean"),
            ("unknown Comment schema", schema + ["properties", "values", "properties", "Comment"], True),
            ("invented Name", schema + ["properties", "values", "properties", "Name"], {"type": "string"}),
            ("invented name alias", schema + ["properties", "values", "properties", "name"], {"type": "string"}),
        ]
        for mutation, path, value in mutations:
            with self.subTest(mutation=mutation):
                records, _, response = self.configuration_records()
                answer = response["payload"]["result"]["structuredContent"]
                node = answer
                for key in path[:-1]:
                    node = node[key]
                if value is None:
                    del node[path[-1]]
                else:
                    node[path[-1]] = value
                with self.assertRaisesRegex(ValueError, "Configuration inspection"):
                    props_driver.audit(records)
        for code in ["not_found", "bad_value"]:
            records, _, response = self.configuration_records()
            response["payload"]["result"].update({"isError": True, "structuredContent": {
                "ok": False, "diagnostics": [{"code": code}]}})
            with self.subTest(code=code), self.assertRaisesRegex(ValueError, "Configuration inspection"):
                props_driver.audit(records)

    def test_audit_rejects_missing_configuration_contract(self):
        records, request, response = self.configuration_records()
        records.remove(request)
        records.remove(response)
        with self.assertRaisesRegex(ValueError, "Configuration inspection"):
            props_driver.audit(records)

    def test_audit_rejects_a_missing_owner_publication(self):
        records = self.accepted_records()
        publications = [record for record in records if record["direction"] == "request"
                        and record["payload"].get("method") == "tools/call"
                        and "executionToken" in record["payload"]["params"]["arguments"]]
        request = publications[-1]
        response = next(record for record in records if record["direction"] == "response"
                        and record["payload"]["id"] == request["payload"]["id"])
        records.remove(request)
        records.remove(response)
        with self.assertRaisesRegex(ValueError, "Properties publication"):
            props_driver.audit(records)

    def test_audit_rejects_configuration_writes_despite_its_discovered_capability(self):
        for mutation in ["preview", "publication"]:
            records, _, _ = self.configuration_records()
            preview = next(record for record in records if record["direction"] == "request"
                           and record["payload"].get("method") == "tools/call"
                           and "ops" in record["payload"]["params"]["arguments"])
            if mutation == "preview":
                preview["payload"]["params"]["arguments"]["at"] = "main:Configuration"
                expected = "outside its fixed owner scope"
            else:
                publication = next(record for record in records if record["direction"] == "request"
                                   and record["payload"].get("method") == "tools/call"
                                   and "executionToken" in record["payload"]["params"]["arguments"])
                response = next(record for record in records if record["direction"] == "response"
                                and record["payload"]["id"] == publication["payload"]["id"])
                response["sourceFilesSha256"] = {**response["sourceFilesSha256"], "src/Configuration.xml": "0" * 64}
                expected = "outside its planned descriptors"
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, expected):
                props_driver.audit(records)

    def test_audit_accepts_contextual_contracts_saved_token_and_honest_provider(self):
        allowed, calls = props_driver.audit(self.accepted_records())
        self.assertEqual(allowed, {"unica.view", "unica.apply", "unica.check"})
        self.assertTrue(calls)

    def test_audit_rejects_successful_structured_answers_flagged_as_mcp_errors(self):
        for observation in ["configuration contract", "preview", "publication", "final view", "final check"]:
            with self.subTest(observation=observation):
                records = copy.deepcopy(self.accepted_records())
                requests = [record for record in records if record["direction"] == "request"
                            and record["payload"].get("method") == "tools/call"]
                if observation == "configuration contract":
                    request = next(record for record in requests
                                   if record["payload"]["params"]["arguments"].get("at") == "main:Configuration.Can.props.set")
                elif observation == "preview":
                    request = next(record for record in requests
                                   if "ops" in record["payload"]["params"]["arguments"])
                elif observation == "publication":
                    request = next(record for record in requests
                                   if "executionToken" in record["payload"]["params"]["arguments"])
                else:
                    tool = "unica.view" if observation == "final view" else "unica.check"
                    request = next(record for record in requests
                                   if record["payload"]["params"]["name"] == tool
                                   and record["payload"]["params"]["arguments"].get("at") == props_driver.ROLE)
                response = next(record for record in records if record["direction"] == "response"
                                and record["payload"]["id"] == request["payload"]["id"])
                self.assertIs(response["payload"]["result"]["structuredContent"]["ok"], True)
                response["payload"]["result"]["isError"] = True
                with self.assertRaisesRegex(ValueError, "MCP error"):
                    props_driver.audit(records)

    def test_audit_rejects_missing_contract_token_swap_source_write_and_alias_loss(self):
        for mutation in ["missing contract", "token swap", "preview writes", "foreign publication", "string boolean", "lost mixed aliases", "abandoned mixed aliases", "provider I/O failure", "sibling target", "unknown outer target"]:
            with self.subTest(mutation=mutation):
                records = copy.deepcopy(self.accepted_records())
                requests = [r for r in records if r["direction"] == "request" and r["payload"].get("method") == "tools/call"]
                preview = next(r for r in requests if "ops" in r["payload"]["params"]["arguments"])
                publication = next(r for r in requests if "executionToken" in r["payload"]["params"]["arguments"])
                if mutation == "missing contract":
                    target = next(r for r in requests if r["payload"]["params"]["arguments"].get("at") == props_driver.ROLE + ".Can.props.set")
                    target["payload"]["params"]["arguments"]["at"] = props_driver.ROLE
                elif mutation == "token swap":
                    publication["payload"]["params"]["arguments"]["executionToken"] = "foreign-token"
                elif mutation in {"preview writes", "foreign publication"}:
                    target = preview if mutation == "preview writes" else publication
                    response = next(r for r in records if r["direction"] == "response" and r["payload"]["id"] == target["payload"]["id"])
                    response["sourceFilesSha256"] = {**response["sourceFilesSha256"], "src/foreign.xml": "0" * 64}
                elif mutation == "string boolean":
                    preview["payload"]["params"]["arguments"]["ops"][0]["args"]["values"]["ClientOrdinaryApplication"] = "false"
                elif mutation == "provider I/O failure":
                    request = next(r for r in requests if r["payload"]["params"]["name"] == "unica.check"
                                   and r["payload"]["params"]["arguments"].get("at") == props_driver.MODULE)
                    response = next(r for r in records if r["direction"] == "response" and r["payload"]["id"] == request["payload"]["id"])
                    response["payload"]["result"]["structuredContent"]["diagnostics"][0]["detailCode"] = "provider_io_error"
                elif mutation in {"sibling target", "unknown outer target"}:
                    preview["payload"]["params"]["arguments"]["ops"][0]["args"]["at"] = props_driver.MODULE
                    preview["payload"]["params"]["arguments"]["at"] = (
                        props_driver.ROLE if mutation == "sibling target" else "main:Configuration")
                else:
                    preview = next(r for r in requests if "ops" in r["payload"]["params"]["arguments"]
                                   and r["payload"]["params"]["arguments"]["at"] == props_driver.ROLE)
                    if mutation == "abandoned mixed aliases":
                        request = copy.deepcopy(preview)
                        response = copy.deepcopy(next(r for r in records if r["direction"] == "response" and r["payload"]["id"] == preview["payload"]["id"]))
                        request["payload"]["id"] = response["payload"]["id"] = 999
                        response["payload"]["result"]["structuredContent"]["data"]["executionToken"] = "abandoned-token"
                        index = records.index(preview)
                        records[index:index] = [request, response]
                    values = preview["payload"]["params"]["arguments"]["ops"][0]["args"]["values"]
                    values["Synonym"] = values.pop("synonym")
                with self.assertRaises(ValueError):
                    props_driver.audit(records)

    def test_shared_evaluate_keeps_default_help_and_source_map_and_accepts_properties_hooks(self):
        for custom in [False, True]:
            with self.subTest(custom=custom), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                workspace = root / "workspace"
                (workspace / "src/cf").mkdir(parents=True)
                (workspace / "src/cf/example.xml").write_text("<example/>")
                fixture = root / "fixture"
                fixture.mkdir()
                for name, content in [("AGENTS.md", "Read {{HELP_PATH}}"), ("prompt.md", "inspect"), ("output.schema.json", "{}")]:
                    (fixture / name).write_text(content)
                packet = root / "packet"
                packet.mkdir()
                binary = packet / "unica"
                binary.write_bytes(b"immutable-test-binary")
                server = types.SimpleNamespace(workspace=workspace, binary=binary, _schemas={"unica.view": {}}, input_schema=lambda _: {})
                calls, help_paths = [], []
                def run(command, prompt, environment, output):
                    calls.append((command, prompt))
                    output.with_suffix(".jsonl").write_text("")
                    output.with_suffix(".stderr").write_text("")
                    (output.parent / "mcp.jsonl").write_text("")
                    return [], output.with_suffix(".jsonl"), output.with_suffix(".stderr")
                package = types.SimpleNamespace(sha256=lambda path: hashlib.sha256(path.read_bytes()).hexdigest(), package_tree_sha256=lambda _: "packet-hash")
                extra = {"help_ref": "skills/meta-edit/SKILL.md", "source_roots": {"main": "src"}} if custom else {}
                with patch.object(dcs_driver, "prepare_packet", return_value=(packet, binary, "packet-hash")), \
                     patch.object(dcs_driver, "packager", return_value=package), \
                     patch.object(dcs_driver, "disabled_skills", return_value=[]), \
                     patch.object(dcs_driver, "codex_command", return_value=["codex-test-double"]), \
                     patch.object(dcs_driver, "isolated_environment", return_value={}), \
                     patch.object(dcs_driver.subprocess, "check_output", return_value="Codex test double"), \
                     patch.object(dcs_driver, "run_cli", side_effect=run), \
                     patch.object(dcs_driver, "stop_owned_daemons"), \
                     patch.object(dcs_driver, "audit_cli", side_effect=lambda events, allowed, help_path: help_paths.append(help_path) or []):
                    dcs_driver.evaluate(server, {"id": "hook-control"}, root / "proof", fixture=fixture,
                                        changed_paths=set(), auditor=lambda _: ({"unica.view"}, []), **extra)
                args = next(arg.split("=", 1)[1] for arg in calls[0][0] if arg.startswith("mcp_servers.unica.args="))
                self.assertEqual(json.loads(json.loads(args)[-1]), {"main": "src"} if custom else {"cf": "src/cf"})
                expected_help = packet / ("skills/meta-edit/SKILL.md" if custom else "references/use-cases/reports-printing.md")
                self.assertEqual(help_paths, [expected_help])
                self.assertIn(str(expected_help), calls[0][1])
