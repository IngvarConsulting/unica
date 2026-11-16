"""Decode observer MCP results without starting a server or touching sources."""
from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import sys
import threading
import unittest
from pathlib import Path
from unittest.mock import Mock

SCRIPT = Path(__file__).resolve().parents[2] / "scripts/dev/unica-observer.py"
SPEC = importlib.util.spec_from_file_location("unica_observer_protocol", SCRIPT)
observer = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = observer
SPEC.loader.exec_module(observer)
AT = "sample:CommonModule.Example.Method.Read.Body"
ENVELOPE = {"ok": True, "at": AT, "summary": "logical node text resolved",
            "data": {"at": AT, "kind": "Body", "title": "Body", "can": []}}


def text_result(text: str) -> dict:
    return {"isError": False, "content": [
        {"type": "text", "text": json.dumps(ENVELOPE)},
        {"type": "text", "text": text},
    ]}


def client_for(*results: dict):
    client = object.__new__(observer.McpClient)
    client.tools = [{"name": "unica.view"}, {"name": "unica.task.result"}]
    client.tool_calls_lock = threading.Lock()
    client.tool_calls = 0
    client.timeout = 10
    client.request = Mock(side_effect=results)
    return client


class ObserverProtocolTests(unittest.TestCase):
    def test_reveal_pages_to_the_exact_address_and_maps_only_body_windows_to_the_parent(self):
        node = shutil.which("node")
        if not node:
            self.skipTest("Node.js is required to exercise the tree navigation")
        script = r"""
const assert = require("node:assert/strict");
const vm = require("node:vm");
const page = require("node:fs").readFileSync(0, "utf8");
const source = page.slice(page.indexOf("function highlightTreeRow"), page.indexOf("function sourceTextHtml"));
const open = new Set();
const rows = new WeakMap();
function row(at, children = [], sourceSet) {
  const selected = new Set();
  const wrapper = {
    dataset: {at, sourceSet},
    firstElementChild: {
      classList: {add: value => selected.add(value), remove: value => selected.delete(value)},
      scrollIntoView: () => assert(selected.has("sel")),
    },
    lastElementChild: {children, querySelector: () => children.find(child => child.dataset.cursor)},
    selected,
  };
  rows.set(wrapper, async () => { open.add(at); });
  return wrapper;
}

const itemParentAt = "main:Report.R.Template.S.Setting.V.Item";
const item = row(itemParentAt + ".Body#L1");
const children = [row(itemParentAt + ".Other")];
const more = {dataset: {cursor: "page2"}, onclick: async () => { children.splice(1, 1, item); }};
children.push(more);
const body = row("main:Report.R.Module.Object.Body");
const owner = row("main:Report.R", [row(itemParentAt, children), body]);
const report = row("main:Report", [owner]);
const root = row("", [row("main:Configuration", [report], "main")]);
const context = vm.createContext({
  $: () => ({firstElementChild: root}),
  expandTreeRows: rows,
  selectedRow: null,
  selectionVersion: 0,
});
vm.runInContext(source, context);
async function check(at, kind, expected, message) {
  const button = {}, status = {};
  await context.revealSelectedInTree(button, status, at, kind);
  assert(expected.selected.has("sel"));
  assert.equal(status.textContent, message);
  assert.equal(button.disabled, false);
}
(async () => {
  await check(item.dataset.at, "Item", item, "");
  assert(open.has(itemParentAt));
  await check(body.dataset.at + "#L1-L2", "Body", body, "В дереве: " + body.dataset.at);
  assert(!item.selected.has("sel"));
})().catch(error => { console.error(error); process.exitCode = 1; });
"""
        result = subprocess.run(
            [node, "-e", script], input=observer.PAGE, text=True, capture_output=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_soap_handler_tree_row_uses_only_the_proven_address(self):
        node = shutil.which("node")
        if not node:
            self.skipTest("Node.js is required for Observer tree checks")
        script = r"""
const assert = require("node:assert/strict");
const vm = require("node:vm");
const page = require("node:fs").readFileSync(0, "utf8");
const source = page.slice(page.indexOf("async function fill("), page.indexOf("function highlightTreeRow"));
let data;
const context = vm.createContext({
  document: {createElement: () => ({remove() {}})},
  ticking: () => () => {},
  view: async () => ({result: {ok: true, data}}),
  makeRow: (label, at) => ({label, at}),
  markLeaf: () => {}, shortLabel: () => "", isGraph: () => false,
});
vm.runInContext(source, context);
(async () => {
  for (const target of ["main:CommonModule.MonthClose.Method.RunScheduled", undefined]) {
    data = {kind: "Operation", at: "main:WebService.Exchange.Operation.Send", props: {implementationAt: target}};
    const rows = [];
    const container = {appendChild: row => rows.push(row), childElementCount: 1};
    await context.fill(container, "main:ScheduledJob.MonthClose");
    const handlers = rows.filter(row => row.label === "Handler");
    assert.equal(handlers.length, target ? 1 : 0);
    if (target) assert.equal(handlers[0].at, target);
  }
})().catch(error => {console.error(error); process.exitCode = 1;});
"""
        result = subprocess.run([node, "-e", script], input=observer.PAGE, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_source_text_preserves_the_envelope_and_exact_text(self):
        for text in ["// Текст\r\nФункция Read()\r\n\tВозврат 1;\r\nКонецФункции", "", "\n\nВозврат 1;\n", '{"ok":false}']:
            with self.subTest(text=text):
                answer = client_for(text_result(text)).call_tool("view", {"at": AT})
                self.assertEqual(answer["result"], ENVELOPE)
                self.assertFalse(answer["isError"])
                self.assertEqual(answer["sourceText"], text)

    def test_structured_success_and_refusal_keep_their_authoritative_envelope(self):
        for envelope in [ENVELOPE, {"ok": False, "summary": "missing", "diagnostics": [{"code": "not_found"}]}]:
            with self.subTest(envelope=envelope):
                error = not envelope["ok"]
                answer = client_for({"isError": error, "structuredContent": envelope,
                                     "content": [{"type": "text", "text": "readable fallback"}]}).call_tool("view", {})
                self.assertEqual(answer["result"], envelope)
                self.assertEqual(answer["isError"], error)
                self.assertNotIn("sourceText", answer)

    def test_plain_refusal_and_malformed_blocks_do_not_become_source_success(self):
        refusal = {"ok": False, "summary": "missing", "diagnostics": [{"code": "not_found"}]}
        answer = client_for({"isError": True, "content": [{"type": "text", "text": json.dumps(refusal)}]}).call_tool("view", {})
        self.assertEqual(answer["result"], refusal)
        self.assertTrue(answer["isError"])
        self.assertNotIn("sourceText", answer)
        malformed = [
            {"content": [{"type": "text", "text": "not JSON"}, {"type": "text", "text": "source"}]},
            {"content": text_result("source")["content"] + [{"type": "text", "text": "extra"}]},
            {"content": [{"type": "text", "text": json.dumps({"ok": True, "data": {"kind": "Catalog"}})},
                         {"type": "text", "text": "unrelated"}]},
        ]
        for result in malformed:
            with self.subTest(result=result):
                answer = client_for(result).call_tool("view", {})
                self.assertFalse(answer["result"]["ok"])
                self.assertNotIn("sourceText", answer)

    def test_final_task_result_keeps_source_text(self):
        queued = {"structuredContent": {"ok": True, "data": {"task": {"status": "working", "taskId": "t"}}}}
        client = client_for(queued, text_result("Возврат 1;\n"))
        answer = client.call_tool("view", {"at": AT})
        self.assertEqual(answer["result"], ENVELOPE)
        self.assertEqual(answer["sourceText"], "Возврат 1;\n")
        self.assertEqual(answer["task"]["polls"], 1)
        self.assertEqual(client.request.call_args.args[1]["name"], "unica.task.result")


def structured(envelope: dict) -> dict:
    return {"isError": not envelope["ok"], "structuredContent": envelope, "content": []}


class ObserverDraftTests(unittest.TestCase):
    def test_operation_without_published_contract_is_named_unavailable(self):
        at = "main:Catalog.Items"
        node = {"ok": True, "at": at, "data": {"at": at, "can": [{"op": "sample.set", "args": "values"}]}}
        refusal = {"ok": False, "summary": "no detailed contract", "diagnostics": [{"code": observer.CONTRACT_UNAVAILABLE_CODE}]}
        draft = observer.Observer(client_for(structured(node), structured(refusal))).draft({"at": at, "op": "sample.set"})
        self.assertEqual(draft["contract"], "unavailable")
        self.assertEqual(draft["note"], observer.CONTRACT_UNAVAILABLE_NOTE)
        self.assertEqual(draft["draft"], {"at": at, "ops": [{"op": "sample.set", "args": {"values": {}}}]})

    def test_order_draft_permutes_the_stored_sequence_across_pages(self):
        at = "main:Subsystem.Sales.Interface.Group.NavigationPanelImportant"
        can = [{"op": "commandOrder.set", "args": "values"}]
        schema = {"type": "object", "additionalProperties": False, "required": ["values"], "properties": {
            "values": {"type": "object", "additionalProperties": False, "required": ["commands"], "properties": {
                "commands": {"type": "array", "minItems": 1, "items": {"type": "string"}}}}}}
        contract = {"argsSchema": schema, "exampleArgs": {"values": {"commands": ["Catalog.Sample.StandardCommand.OpenList"]}}}
        page = lambda names, cursor=None: structured({"ok": True, "at": at, **({"cursor": cursor} if cursor else {}),
                                                      "data": {"at": at, "can": can, "items": [{"command": name} for name in names]}})
        client = client_for(
            page(["A"], "next"),
            structured({"ok": True, "at": f"{at}.Can.commandOrder.set", "data": {"props": {"contract": contract}}}),
            page(["A", "B"], "next"),
            page(["C"]),
        )
        draft = observer.Observer(client).draft({"at": at, "op": "commandOrder.set"})
        self.assertEqual(draft["draft"]["ops"][0]["args"], {"values": {"commands": ["A", "B", "C"]}})
        self.assertEqual(draft["validation"], [])
        self.assertEqual(
            [call.args[1]["arguments"] for call in client.request.call_args_list],
            [{"at": at}, {"at": f"{at}.Can.commandOrder.set"}, {"at": at}, {"at": at, "cursor": "next"}],
        )

    def test_stored_order_refuses_a_repeated_cursor(self):
        at = "main:Subsystem.Sales.Interface.Group"
        page = structured({"ok": True, "at": at, "cursor": "same", "data": {"at": at, "items": [{"title": "A"}]}})
        with self.assertRaisesRegex(observer.ApiError, "repeats cursor"):
            observer.Observer(client_for(page, page)).stored_sequence(at, "title")
