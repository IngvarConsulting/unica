"""Acceptance corpus: real developer tasks ride the canonical surface.

The common corpus has source, pinned-analyzer delivery and real-agent drivers.
Runtime drivers are implemented with their surface tasks;
a profile without an executable driver is rejected.

Every scenario of the corpus (the closed block S001-S440 in
tests/fixtures/acceptance/scenario-corpus.json and one file per later scenario
in tests/fixtures/acceptance/scenarios/, both loaded by
scripts/ci/acceptance_corpus.py) is a real configuration-development task
expressed as a wire of canonical unica.* calls against the acceptance fixture
workspace.  The corpus freezes, per step, the class of answer the surface
gives today:

  ok          ok:true result without a task receipt
  task        ok:true queued/working Task receipt
  unsupported typed refusal whose code starts with unsupported_, or
              target_kind_unsupported
  provider    typed provider_unavailable refusal
  cancelled   typed task_cancelled outcome of an intended cancellation
  failed      typed task_failed terminal outcome
  refused     an intended typed refusal: a code from the closed dictionary in
              crates/unica-refusal/src/lib.rs whose default outcome
              is fixCall or fixSource, or invalid_state (needsHuman);
              postcondition_failed is a product defect, not a refusal
  gap         a documented non-passable spot with its reason

A wire is passable when every step answers its frozen class; a raw error, an
unknown tool, or an undocumented typed refusal fails the run.  A step marked
`form: true` is sent the way a form client sends it: every published schema
default fills an argument the step leaves out.  Environment-shaped
steps (docs search, platform runs, task follow-ups) freeze the set of classes
the environment legitimately selects from.
"""
from __future__ import annotations

from scripts.ci.prepared_binary import binary_path, ensure_binary

import importlib.util
import json
import queue
import re
import threading
import time
from contextlib import ExitStack
import os
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

from tests.ci.acceptance_controls import (
    capture_values,
    content_mismatches,
    corpus_storage,
    file_value,
    load_acceptance_corpus,
    matches_step,
    substitute_capture,
    relative_file,
    tool_result,
    validate_controls,
)

REPO_ROOT = Path(__file__).resolve().parents[2]
BINARY = binary_path(REPO_ROOT)

from tests.ci.form_runtime_diagnostics import WORKSPACE as FORM_RUNTIME_WORKSPACE

from tests.ci.acceptance_profiles import (
    DIAGNOSTICS_WORKSPACE, SOURCE_WORKSPACES, SYMBOL_WORKSPACE, completed_delivery_call, isolated_environment,
    select_profile,
)

REFUSAL_DICTIONARY = REPO_ROOT / "crates/unica-refusal/src/lib.rs"

# Outcomes that send the next move to the caller: its arguments or its
# sources.  A code that answers one of them says the request has to change,
# which is what a `refused` step freezes.
CALLER_OUTCOMES = {"FixCall", "FixSource"}
# Of the needsHuman codes only invalid_state is a refusal of the request (the
# workspace state forbids it); the rest, such as rollback_incomplete or
# task_backend_failed, are failures of the environment, not refusals.
NEEDS_HUMAN_REFUSALS = {"invalid_state"}
# Unsupported answers whose code lacks the `unsupported_` prefix; they belong
# to the `unsupported` class like the prefixed ones.
UNSUPPORTED_CODES = {"target_kind_unsupported"}
# Caller-side codes that are still not refusals of the request:
# provider_unavailable has its own class, and postcondition_failed is the
# product failing its own postcondition — a defect to document as a gap,
# never an intended refusal.
NOT_REFUSALS = {"provider_unavailable", "postcondition_failed"}


def is_unsupported(code: str) -> bool:
    return code.startswith("unsupported_") or code in UNSUPPORTED_CODES


def _impl_function(impl_body: str, name: str) -> str:
    match = re.search(
        rf"^    pub const fn {name}\(self\)[^{{]*\{{\n(.*?)^    \}}$",
        impl_body,
        re.MULTILINE | re.DOTALL,
    )
    if match is None:
        raise AssertionError(f"RefusalCode::{name} not found in {REFUSAL_DICTIONARY}")
    return match.group(1)


def refusal_dictionary() -> dict[str, str]:
    """The closed refusal dictionary of the canonical surface, read from the
    one place that defines it: wire name -> default outcome variant.

    The harness keeps no copy of its own, so a code the product adds or
    removes reaches the corpus classes with the same change (#1340)."""
    text = REFUSAL_DICTIONARY.read_text(encoding="utf-8")
    impl = re.search(r"^impl RefusalCode \{\n(.*?)^\}$", text, re.MULTILINE | re.DOTALL)
    if impl is None:
        raise AssertionError(f"impl RefusalCode not found in {REFUSAL_DICTIONARY}")
    body = impl.group(1)
    names = dict(re.findall(r'Self::(\w+) => "(\w+)",', _impl_function(body, "as_str")))
    outcomes = dict(
        re.findall(r"Self::(\w+) => Outcome::(\w+),", _impl_function(body, "outcome"))
    )
    declared = re.findall(r"Self::(\w+),", body.split("pub const ALL:", 1)[-1])
    if not names or set(names) != set(outcomes) or set(names) != set(declared):
        raise AssertionError(
            f"{REFUSAL_DICTIONARY}: as_str, outcome and ALL disagree on the variants"
        )
    return {names[variant]: outcomes[variant] for variant in names}


def caller_refusal_codes(dictionary: dict[str, str]) -> set[str]:
    """Codes a `refused` step may freeze: every dictionary code whose default
    outcome is fixCall or fixSource, plus the named needsHuman refusals, minus
    those another class owns."""
    return {
        code
        for code, outcome in dictionary.items()
        if (outcome in CALLER_OUTCOMES or code in NEEDS_HUMAN_REFUSALS)
        and not is_unsupported(code)
        and code not in NOT_REFUSALS
    }


REFUSAL_CODES = caller_refusal_codes(refusal_dictionary())

EXPECT_CLASSES = {
    "ok",
    "task",
    "unsupported",
    "provider",
    "cancelled",
    "failed",
    "refused",
    "gap",
}


def substitute(value, context):
    if isinstance(value, str) and value.startswith("$capture."):
        return substitute_capture(value, context)
    if isinstance(value, str):
        if value.startswith("$workspace/"):
            relative = relative_file(value[len("$workspace/"):])
            if "workspace" not in context:
                raise ValueError("absolute workspace path needs an active scenario")
            return str(Path(context["workspace"]).joinpath(*relative.parts))
        if value == "$task":
            return context.get("task", "00000000-0000-4000-8000-000000000000")
        if value == "$executionToken":
            return context.get("executionToken", "unica-missing-execution-token")
        if value == "$rev":
            return context.get("rev", "unica-missing-rev")
        return value
    if isinstance(value, list):
        return [substitute(item, context) for item in value]
    if isinstance(value, dict):
        return {key: substitute(item, context) for key, item in value.items()}
    return value


def classify(response, context):
    if response is None:
        return "error", "no response"
    if "error" in response:
        message = str((response.get("error") or {}).get("message", ""))
        # The daemon's own typed answer when a provider (documentation,
        # search index) does not respond within its deadline. It is not a
        # transport failure: the tool ran and its provider was unavailable,
        # which the environment-shaped steps freeze as `provider`.
        if "deadline expired" in message:
            return "provider", message[:160]
        return "error", json.dumps(response, ensure_ascii=False)[:200]
    structured = tool_result(response)
    if structured is None:
        return "error", "no structuredContent"
    if structured.get("rev"):
        context["rev"] = structured["rev"]
    data = structured.get("data") or {}
    if data.get("executionToken"):
        context["executionToken"] = data["executionToken"]
    task = data.get("task") or {}
    if task.get("taskId"):
        context["task"] = task["taskId"]
    if structured.get("ok"):
        if task.get("status") in {"queued", "working"}:
            return "task", task.get("status", "")
        return "ok", structured.get("summary", "")
    diagnostics = structured.get("diagnostics") or [{}]
    code = diagnostics[0].get("code", "<none>")
    message = diagnostics[0].get("message", "")
    if is_unsupported(code):
        return "unsupported", code
    if code == "provider_unavailable":
        return "provider", message[:160]
    if code == "task_cancelled":
        return "cancelled", message[:160]
    if code == "task_failed":
        return "failed", message[:160]
    if code in REFUSAL_CODES:
        # Typed refusals from the closed code set: the caller's request, not
        # the surface, is what has to change.
        return "refused", f"{code}: {message[:160]}"
    return "gap-candidate", f"{code}: {message[:200]}"



FORMAT_WORKSPACE = "tests/fixtures/acceptance/workspace-format"
# A directory that is not a 1C workspace at all: no v8project.yaml, no
# autodetected source roots.  Scenarios there freeze what the surface
# answers before any source set is admitted.
BARE_WORKSPACE = "tests/fixtures/acceptance/workspace-bare"


def derive_source_sets(source: Path, workspace: Path) -> None:
    """Derive the format-probe source sets of `workspace-format/` from `src`.

    `v8project.yaml` of that fixture declares three sets and no `main`:
    `newer` is the same tree with every 2.20 root rewritten to 2.21 (and the
    Reports and XDTO packages dropped, because the strict read port cannot
    open a 2.21 template wrapper), `older` is the same tree at 2.19 — the
    dump a platform before 8.3.27 wrote, which reads but cannot be edited —
    `nosupport` is `src` without
    `Ext/ParentConfigurations.bin`, and `unversioned` is `src` whose
    Configuration root carries no version attribute. Deriving them here keeps
    one copy of the platform XML in the repository. The probes live in their
    own workspace because `find` walks every identity of every declared set
    on each call, and the scenarios of the default workspace freeze `find` as
    an immediate result.
    """

    def copy(name: str, rewrite, drop_bin: bool = False, drop_dirs=()):
        target = workspace / name
        shutil.copytree(source, target)
        for directory in drop_dirs:
            shutil.rmtree(target / directory, ignore_errors=True)
        if drop_bin:
            (target / "Ext/ParentConfigurations.bin").unlink()
        for path in target.rglob("*.xml"):
            raw = path.read_bytes()
            bom = raw.startswith(b"\xef\xbb\xbf")
            text = raw.decode("utf-8-sig")
            rewritten = rewrite(path, text)
            if rewritten != text:
                path.write_bytes((b"\xef\xbb\xbf" if bom else b"") + rewritten.encode("utf-8"))

    def newer(path: Path, text: str) -> str:
        text = text.replace('version="2.20"', 'version="2.21"')
        if path.name == "Configuration.xml":
            text = re.sub(r"<(Report|XDTOPackage)>[^<]+</\1>", "", text)
        return text

    def older(path: Path, text: str) -> str:
        text = text.replace('version="2.20"', 'version="2.19"')
        if path.name == "Configuration.xml":
            text = re.sub(r"<(Report|XDTOPackage)>[^<]+</\1>", "", text)
        return text

    def unversioned(path: Path, text: str) -> str:
        if path.name == "Configuration.xml":
            text = re.sub(r'(<MetaDataObject[^>]*?) version="2\.20"', r"\1", text, count=1)
        return text

    copy("src-newer", newer, drop_dirs=("Reports", "XDTOPackages"))
    copy("src-older", older, drop_dirs=("Reports", "XDTOPackages"))
    copy("src-nosupport", lambda path, text: text, drop_bin=True)
    copy("src-unversioned", unversioned)

def form_arguments(schema, arguments):
    """What a form client such as MCP Inspector sends: every published
    top-level `default` fills an argument the step leaves out (#1210)."""
    filled = dict(arguments)
    for name, property_schema in (schema.get("properties") or {}).items():
        if name not in filled and "default" in property_schema:
            filled[name] = property_schema["default"]
    return filled


def scenario_publishes(scenario) -> bool:
    """True when a step can change the workspace: an apply that is not a preview."""
    return any(
        step["tool"] == "unica.apply" and "executionToken" in step["args"]
        for step in scenario["wire"]
    )


FORM_KINDS = {"Form", "CommonForm"}


def names_form(address: str) -> bool | None:
    """True when an address ends on a form (`main:Catalog.X.Form.Y`), None
    when it names a node inside one, False when no form is on its path."""
    segments = re.split(r"(?<!\\)\.", address.partition(":")[2])
    if len(segments) >= 2 and segments[-2] in FORM_KINDS:
        return True
    return None if FORM_KINDS & set(segments[:-2]) else False


def pins_published_file(assertion) -> bool:
    """True when a file assertion fixes what the publication wrote: exact
    bytes, XML content or a removed path.  `notEq`, a still-present path or
    the digest of an earlier capture tell nothing about the new state."""
    if assertion["kind"] == "exists":
        return assertion.get("eq") is False
    if assertion["kind"] == "sha256":
        return isinstance(assertion.get("eq"), str) and not assertion["eq"].startswith("$")
    return assertion["kind"] in {"xml", "xmlTags"} and ("eq" in assertion or "contains" in assertion)


def apply_operation_cases(scenarios, registry) -> dict[tuple[str, str], list[str]]:
    """Map every registered `(operation, family)` to the scenarios that
    publish it: an `unica.apply` preview freezes exactly `ok`, and a later
    step executes its token (`$executionToken` of the last plan or the
    `$capture` of its `/data/executionToken`), freezes exactly `ok` and pins
    the published files.  A refusal, a gap, an unfinished task or a preview that
    nobody publishes does not run the writer, so it is not a case.  A name
    that several families register is told apart by the kind of its target:
    a form owns its own `attribute.add` and `command.add`."""
    families: dict[str, list[tuple[str, str]]] = {}
    for operation, family, target, _ in registry:
        families.setdefault(operation, []).append((family, target))
    cases: dict[tuple[str, str], list[str]] = {}
    for scenario in scenarios:
        # `$executionToken` is the token of the last preview that returned a
        # plan; a refusal returns none and keeps it.
        plans: dict[str, list[tuple[str, str]]] = {}
        for step in scenario["wire"]:
            if step["tool"] != "unica.apply":
                continue
            token = step["args"].get("executionToken")
            if token is not None:
                # A refused execution leaves the plan and its token in place.
                if step["expect"] != ["ok"]:
                    continue
                planned = plans.pop(token, [])
                if any(pins_published_file(assertion) for assertion in step.get("fileAssertions") or []):
                    for key in planned:
                        ids = cases.setdefault(key, [])
                        if scenario["id"] not in ids:
                            ids.append(scenario["id"])
                continue
            if step["expect"] != ["ok"]:
                continue
            planned = []
            for operation in step["args"].get("ops") or []:
                name = operation["op"]
                candidates = families.get(name, [])
                if len(candidates) > 1:
                    target = (operation.get("args") or {}).get("at") or step["args"]["at"]
                    on_form = names_form(target)
                    candidates = [row for row in candidates if (row[1] == "Form") == on_form]
                    if on_form is None or len(candidates) != 1:
                        raise ValueError(f"{scenario['id']}: `{name}` at `{target}` matches no single family")
                planned.extend((name, family) for family, _ in candidates)
            plans["$executionToken"] = planned
            for name, descriptor in (step.get("captures") or {}).items():
                if descriptor.get("pointer") == "/data/executionToken":
                    plans[f"$capture.{name}"] = planned
    return cases


def matches(expected: list[str], actual: str) -> bool:
    if actual in expected:
        return True
    # A documented gap freezes today's non-passable answer; a refusal probe
    # freezes an intended typed refusal.  Both arrive through the same classes.
    if "gap" in expected and actual in {"refused", "gap-candidate", "provider"}:
        return True
    return False


RESPONSE_TIMEOUT_SECONDS = 120.0


class AcceptanceServer:
    """One JSON-RPC session with the daemon over stdio. Responses are read by
    a helper thread so a silent server fails the step instead of hanging the
    job until an external timeout."""

    def __init__(self, cwd: Path, state: Path, protocol: str, environment=None):
        self.binary = BINARY
        self.workspace = cwd
        env = dict(os.environ if environment is None else environment)
        env["UNICA_PROVIDER_STATE_DIR"] = str(state)
        # Демон переживает сессию: без назначенной паузы он остаётся на
        # четверть часа, а сценариев в корпусе десятки — к концу набора их
        # столько же, и подключение начинает отказывать.
        env["UNICA_DAEMON_IDLE_GRACE_MS"] = "5000"
        self.provider_failure_cache_is_default = "UNICA_CACHE_DIR" not in env
        self.stderr_path = state / "unica-stderr.log"
        self.stderr_file = open(self.stderr_path, "wb")  # noqa: SIM115 - lives as long as the process
        self.process = subprocess.Popen(
            [str(self.binary)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr_file,
            cwd=cwd,
            env=env,
        )
        self.next_id = 0
        self.lines: "queue.Queue[bytes | None]" = queue.Queue()
        self.reader = threading.Thread(target=self._pump, daemon=True)
        self.reader.start()
        self.label = "initialize"
        self.send(
            {
                "jsonrpc": "2.0",
                "id": self.request_id(),
                "method": "initialize",
                "params": {
                    "protocolVersion": protocol,
                    "capabilities": {},
                    "clientInfo": {"name": "acceptance-corpus", "version": "0"},
                },
            }
        )
        self.receive()
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def request_id(self) -> int:
        self.next_id += 1
        return self.next_id

    def send(self, payload) -> None:
        self.process.stdin.write((json.dumps(payload) + "\n").encode())
        self.process.stdin.flush()

    def _pump(self) -> None:
        stream = self.process.stdout
        assert stream is not None
        for line in iter(stream.readline, b""):
            self.lines.put(line)
        self.lines.put(None)

    def stderr_tail(self, limit: int = 1200) -> str:
        try:
            self.stderr_file.flush()
            return self.stderr_path.read_text(encoding="utf-8", errors="replace")[-limit:]
        except OSError:
            return ""

    def receive(self, expected_id: int | None = None, timeout=None):
        deadline = time.monotonic() + timeout if timeout is not None else None
        while True:
            try:
                remaining = deadline - time.monotonic() if deadline else RESPONSE_TIMEOUT_SECONDS
                if remaining <= 0:
                    raise queue.Empty
                line = self.lines.get(timeout=remaining)
            except queue.Empty:
                self.close()
                raise TimeoutError(
                    f"no response from unica within {RESPONSE_TIMEOUT_SECONDS:.0f}s "
                    f"while running {self.label}; stderr tail: {self.stderr_tail()}"
                ) from None
            if line is None:
                return None
            line = line.strip()
            if line:
                payload = json.loads(line)
                if "id" in payload and (expected_id is None or payload["id"] == expected_id):
                    return payload

    def call(self, tool: str, arguments, label: str | None = None, *, timeout=None):
        self.label = label or tool
        request_id = self.request_id()
        self.send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "tools/call",
                "params": {"name": tool, "arguments": arguments},
            }
        )
        return self.receive(request_id, timeout)

    def input_schema(self, tool: str):
        """The published inputSchema of one tool, read from tools/list."""
        if not hasattr(self, "_schemas"):
            self.label = "tools/list"
            request_id = self.request_id()
            self.send({"jsonrpc": "2.0", "id": request_id, "method": "tools/list"})
            response = self.receive(request_id) or {}
            self._schemas = {
                entry["name"]: entry.get("inputSchema") or {}
                for entry in (response.get("result") or {}).get("tools") or []
            }
        if tool not in self._schemas:
            # A missing schema would turn a form step into a plain call
            # that proves nothing about published defaults.
            raise ValueError(f"tools/list did not publish {tool}")
        return self._schemas[tool]

    def close(self) -> None:
        try:
            self.process.stdin.close()
        except OSError:
            pass
        self.process.terminate()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.reader.join(timeout=10)
        if self.reader.is_alive():
            raise TimeoutError("source stdout reader did not stop after MCP exit")
        self.process.stdout.close()
        try:
            self.stderr_file.close()
        except OSError:
            pass


def materialize_external_form_link(workspace: Path) -> None:
    """Create the fixed adversarial payload in an isolated corpus copy."""
    payload = workspace / "epf/Import/Forms/Main/Ext/Form.xml"
    target = workspace / "epf/Other/Forms/Main/Ext/Form.xml"
    relative_target = "../../../../Other/Forms/Main/Ext/Form.xml"
    placeholder = b"<!-- replaced by a real relative symlink before MCP starts -->\n"
    if payload.is_symlink() or payload.read_bytes() != placeholder or not target.is_file():
        raise ValueError("external link fixture is not the declared regular placeholder")
    payload.unlink()
    payload.symlink_to(relative_target)
    if not payload.is_symlink() or os.readlink(payload) != relative_target:
        raise ValueError("external link fixture did not create an actual relative symlink")
    if payload.resolve(strict=True) != target.resolve(strict=True):
        raise ValueError("external link fixture does not point to the neighboring owner")


class ExternalLinkFixtureTests(unittest.TestCase):
    def test_fixed_link_is_real_and_the_neighbor_bytes_are_preserved(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw) / "workspace"
            shutil.copytree(REPO_ROOT / "tests/fixtures/acceptance/workspace-external-linked", workspace)
            target = workspace / "epf/Other/Forms/Main/Ext/Form.xml"
            original = target.read_bytes()
            materialize_external_form_link(workspace)
            payload = workspace / "epf/Import/Forms/Main/Ext/Form.xml"
            self.assertTrue(payload.is_symlink())
            self.assertEqual(os.readlink(payload), "../../../../Other/Forms/Main/Ext/Form.xml")
            self.assertEqual(payload.resolve(strict=True), target.resolve(strict=True))
            self.assertEqual(target.read_bytes(), original)
            with self.assertRaises(ValueError):
                materialize_external_form_link(workspace)


    def test_link_refusal_cannot_pass_as_wrong_descriptor_identity(self):
        corpus = load_acceptance_corpus()
        case = next(s for s in corpus["scenarios"] if s.get("name") == "external-linked-payload-refusal")
        link = {"assertions": case["wire"][0]["assertions"]}
        foreign = case["wire"][1]["assertions"][1]["eq"]
        wrong_cause = {"ok": False, "diagnostics": foreign}
        mismatches = content_mismatches(link, wrong_cause, "provider", "")
        self.assertEqual(len(mismatches), 1)
        self.assertIn("notEq", mismatches[0])


class SavedPlanWireTests(unittest.TestCase):
    def test_capture_values_are_opaque_even_when_they_look_like_template_tokens(self):
        context = {"captures": {"text": "$executionToken", "object": {"at": "$rev"}},
                   "executionToken": "saved-plan", "rev": "current-revision"}
        self.assertEqual(substitute({"text": "$capture.text", "object": "$capture.object"}, context),
                         {"text": "$executionToken", "object": {"at": "$rev"}})

    def test_absolute_path_query_stays_inside_the_scenario_workspace(self):
        context = {"workspace": Path("/scenario")}
        self.assertEqual(substitute({"path": "$workspace/src/Form.xml"}, context),
                         {"path": str(Path("/scenario/src/Form.xml"))})
        for path in ["$workspace/../outside.xml", "$workspace/C:/outside.xml", "$workspace//outside.xml"]:
            with self.subTest(path=path), self.assertRaises(ValueError):
                substitute(path, context)
        with self.assertRaises(ValueError):
            substitute("$workspace/src/Form.xml", {})
        self.assertEqual(substitute("$capture.literal", {"captures": {"literal": "$workspace/src/Form.xml"}}),
                         "$workspace/src/Form.xml")

    def test_execute_uses_the_preview_token_and_reads_do_not_replace_it(self):
        context = {}
        plan = {"result": {"structuredContent": {
            "ok": True, "rev": "source-revision", "data": {"executionToken": "saved-plan-token"}
        }}}
        self.assertEqual(classify(plan, context)[0], "ok")
        classify({"result": {"structuredContent": {"ok": True, "rev": "read-revision"}}}, context)
        self.assertEqual(substitute({"executionToken": "$executionToken"}, context),
                         {"executionToken": "saved-plan-token"})
        self.assertEqual(context["rev"], "read-revision")

    def test_only_token_execution_requires_an_isolated_mutable_workspace(self):
        self.assertFalse(scenario_publishes({"wire": [{"tool": "unica.apply", "args": {
            "at": "main:Configuration", "ops": []
        }}]}))
        self.assertTrue(scenario_publishes({"wire": [{"tool": "unica.apply", "args": {
            "executionToken": "$executionToken"
        }}]}))


class AcceptanceCorpusShapeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.corpus = load_acceptance_corpus()

    def test_corpus_ids_are_stable_and_the_manifest_freezes_every_step(self) -> None:
        storage = corpus_storage()
        scenarios = self.corpus["scenarios"]
        identifiers = [scenario["id"] for scenario in scenarios]
        self.assertEqual(len(identifiers), len(set(identifiers)), "scenario ids are unique")
        # The historical block keeps its numbers and is closed: links to S###
        # still find their scenario, and nothing takes the next number.
        legacy = [identifier for identifier in identifiers if storage.LEGACY_ID.fullmatch(identifier)]
        self.assertEqual(legacy, [f"S{index:03d}" for index in range(1, storage.LEGACY_LAST + 1)])
        for identifier in identifiers[len(legacy):]:
            storage.stable_order(identifier)
        # A dropped scenario or step is caught per scenario, not by a total
        # every addition would have to edit.
        self.assertEqual(
            storage.manifest_problems(self.corpus, storage.read_manifest()), [],
            "the corpus differs from tests/fixtures/acceptance/scenario-manifest/",
        )

    def test_all_scenarios_have_an_executable_profile(self) -> None:
        source = {s["id"] for s in select_profile(self.corpus, "source")["scenarios"]}
        delivery = {s["id"] for s in select_profile(self.corpus, "delivery")["scenarios"]}
        agent = {s["id"] for s in select_profile(self.corpus, "agent-evaluation")["scenarios"]}
        fault = {s["id"] for s in select_profile(self.corpus, "fault-injection")["scenarios"]}
        self.assertFalse(fault & (source | delivery | agent))
        self.assertFalse(source & delivery or source & agent or delivery & agent)
        self.assertEqual(source | delivery | agent | fault, {s["id"] for s in self.corpus["scenarios"]})
        # The closed S### block keeps its profiles; a per-file scenario
        # declares its own and is admitted by select_profile, so two added
        # scenarios never edit these sets.
        legacy = corpus_storage().LEGACY_ID.fullmatch
        self.assertEqual({i for i in fault if legacy(i)}, {"S344", "S345", "S346", "S389", "S390", "S396", "S398"})
        self.assertEqual({i for i in agent if legacy(i)}, {"S328", "S335"})
        self.assertEqual({i for i in delivery if legacy(i)}, {'S324', 'S339', 'S340', 'S341', 'S342', 'S343', 'S359'})

    def test_every_step_freezes_known_classes_and_documents_gaps(self) -> None:
        for scenario in self.corpus["scenarios"]:
            declared = set()
            workspace = scenario.get("workspace", self.corpus["workspace"])
            self.assertIn(
                workspace,
                SOURCE_WORKSPACES | {SYMBOL_WORKSPACE, DIAGNOSTICS_WORKSPACE, FORM_RUNTIME_WORKSPACE},
                f"{scenario['id']}: a scenario runs on one of the registered fixture workspaces",
            )
            for index, step in enumerate(scenario["wire"]):
                with self.subTest(scenario=scenario["id"], step=index):
                    validate_controls(step, declared)
                    self.assertTrue(step["tool"].startswith("unica."))
                    expected = step["expect"]
                    self.assertTrue(expected, "every step freezes an expectation")
                    self.assertTrue(set(expected) <= EXPECT_CLASSES, expected)
                    if "form" in step:
                        self.assertIs(step["form"], True, "a form step is marked `form: true`")
                    if "gap" in expected:
                        self.assertTrue(
                            step.get("gap"),
                            "a documented gap names its reason",
                        )
                    if "refused" in expected:
                        refusal = step.get("refusal") or ""
                        self.assertTrue(refusal, "an intended refusal freezes its message")
                        code = refusal.split(":", 1)[0]
                        self.assertIn(
                            code,
                            REFUSAL_CODES,
                            "a refusal is frozen as `code: message` with a code from the closed set",
                        )
                    if step["tool"] == "unica.check" and step["args"].get("at") and expected == ["ok"]:
                        # A check over a node freezes the verdict it expects and
                        # the validators the node kind owns; the caller never
                        # names a validator, so the corpus is where the
                        # kind-to-validator table is witnessed.
                        self.assertIn(
                            step.get("status"),
                            {"passed", "failed", "readable"},
                            "a check over a node freezes the verdict it expects",
                        )
                        self.assertIsInstance(
                            step.get("validators"),
                            list,
                            "a check over a node freezes the validators its kind owns",
                        )

    def test_refusal_classes_come_from_the_product_dictionary(self) -> None:
        def classify_diagnostic(code):
            response = {"result": {"structuredContent": {
                "ok": False, "diagnostics": [{"code": code, "message": "m"}]}}}
            return classify(response, {})

        # The harness reads the closed dictionary instead of keeping a copy:
        # every variant is found, and a legitimate caller-side refusal such
        # as `view`'s foreign cursor (#1340) is a `refused`, not a gap.
        dictionary = refusal_dictionary()
        declared = re.search(
            r"pub const ALL: \[Self; (\d+)\]",
            REFUSAL_DICTIONARY.read_text(encoding="utf-8"),
        )
        self.assertIsNotNone(declared)
        assert declared is not None
        self.assertEqual(len(dictionary), int(declared.group(1)))
        self.assertLessEqual(REFUSAL_CODES, set(dictionary))
        self.assertLessEqual(
            {"bad_value", "not_found", "invalid_state", "invalid_source",
             "stale_revision", "invalid_cursor"},
            REFUSAL_CODES,
        )
        self.assertEqual(dictionary["invalid_cursor"], "FixCall")
        self.assertEqual(dictionary["invalid_state"], "NeedsHuman")
        # Environment failures are not refusals of the request.
        for failure in ("rollback_incomplete", "task_backend_failed"):
            self.assertIn(failure, dictionary)
            self.assertNotIn(failure, REFUSAL_CODES)
        for code in REFUSAL_CODES:
            self.assertTrue(
                dictionary[code] in CALLER_OUTCOMES or code in NEEDS_HUMAN_REFUSALS,
                code,
            )
            # `unsupported` and `provider` own their codes before `refused`.
            self.assertFalse(code.startswith("unsupported_"), code)
        self.assertIn("target_kind_unsupported", dictionary)
        self.assertNotIn("target_kind_unsupported", REFUSAL_CODES)
        self.assertEqual(
            classify_diagnostic("target_kind_unsupported")[0], "unsupported"
        )
        # A failed product postcondition is a defect, never an intended refusal.
        self.assertEqual(dictionary["postcondition_failed"], "FixSource")
        self.assertNotIn("postcondition_failed", REFUSAL_CODES)
        self.assertEqual(
            classify_diagnostic("postcondition_failed")[0], "gap-candidate"
        )
        self.assertNotIn("provider_unavailable", REFUSAL_CODES)

    def test_gaps_stay_a_bounded_exception_not_a_habit(self) -> None:
        gap_steps = [
            (scenario["id"], step)
            for scenario in self.corpus["scenarios"]
            for step in scenario["wire"]
            if "gap" in step["expect"]
        ]
        # Every documented gap names a reproduced surface defect in its text.
        # The rendered registry lists them; the ceiling from the fixture README
        # (two) is a ratchet against silent growth, not a target.
        self.assertLessEqual(
            len(gap_steps),
            2,
            "documented gaps grew: either fix the surface or re-approve the corpus",
        )


class AcceptanceRegistryDocumentTests(unittest.TestCase):
    """The scenario registry is rendered from the corpus when the site is
    built; it is not committed, so two added scenarios never edit its counts
    and coverage lists in the same lines."""

    METADATA_CONTEXT = [
        ("ObjectBorrow", "object.borrow", "ExtensionRoot", "From"),
        ("ObjectCreate", "object.create", "Configuration", "Values"),
        ("ObjectRemove", "object.remove", "MetadataOrCommonTemplate", "Target"),
        ("ContentSet", "content.set", "ExchangePlan", "Items"),
        ("RelationAdd", "relation.add", "Metadata", "Values"),
        ("RelationRemove", "relation.remove", "Metadata", "Values"),
        ("RelationReplace", "relation.replace", "Metadata", "Values"),
        ("HelpCreate", "help.create", "Metadata", "Values"),
        ("AttributeAdd", "attribute.add", "MetadataAttributeContainer", "Items"),
        ("AttributeSet", "attribute.set", "MetadataAttribute", "Values"),
        ("AttributeRemove", "attribute.remove", "MetadataAttribute", "Target"),
        ("TabularSectionAdd", "tabularSection.add", "MetadataTabularSections", "Items"),
        ("TabularSectionSet", "tabularSection.set", "MetadataTabularSection", "Values"),
        ("TabularSectionRemove", "tabularSection.remove", "MetadataTabularSection", "Target"),
        ("DimensionAdd", "dimension.add", "MetadataDimensions", "Items"),
        ("DimensionSet", "dimension.set", "MetadataDimension", "Values"),
        ("DimensionRemove", "dimension.remove", "MetadataDimension", "Target"),
        ("ResourceAdd", "resource.add", "MetadataResources", "Items"),
        ("ResourceSet", "resource.set", "MetadataResource", "Values"),
        ("ResourceRemove", "resource.remove", "MetadataResource", "Target"),
        ("EnumValueAdd", "enumValue.add", "MetadataEnumValues", "Items"),
        ("EnumValueSet", "enumValue.set", "MetadataEnumValue", "Values"),
        ("EnumValueRemove", "enumValue.remove", "MetadataEnumValue", "Target"),
        ("ColumnAdd", "column.add", "MetadataColumns", "Items"),
        ("ColumnSet", "column.set", "MetadataColumn", "Values"),
        ("ColumnRemove", "column.remove", "MetadataColumn", "Target"),
        ("TemplateAdd", "template.add", "MetadataTemplates", "Items"),
        ("TemplateSet", "template.set", "MetadataTemplate", "Values"),
        ("TemplateRemove", "template.remove", "MetadataTemplate", "Target"),
        ("CommandAdd", "command.add", "MetadataCommands", "Items"),
        ("CommandSet", "command.set", "MetadataCommand", "Values"),
        ("CommandRemove", "command.remove", "MetadataCommand", "Target"),
        ("PredefinedItemAdd", "predefinedItem.add", "MetadataPredefinedItems", "Items"),
        ("PredefinedItemSet", "predefinedItem.set", "MetadataPredefinedItem", "Values"),
        ("PredefinedItemRemove", "predefinedItem.remove", "MetadataPredefinedItem", "Target"),
    ]
    FORM_CONTEXT = [
        ("Add", "form.add", "MetadataForms", "Items"),
        ("Set", "form.set", "MetadataForms", "Values"),
        ("Remove", "form.remove", "Form", "Target"),
        ("ItemAdd", "item.add", "FormItemContainer", "Items"),
        ("ItemRemove", "item.remove", "FormItem", "Target"),
        ("ItemMove", "item.move", "FormItem", "Values"),
        ("AttributeAdd", "attribute.add", "Form", "Items"),
        ("CommandAdd", "command.add", "Form", "Items"),
        ("EventBind", "event.bind", "FormEvent", "Values"),
        ("PropsSet", "props.set", "Form", "Values"),
    ]
    CONTEXT_ROWS = [
        (name, "Metadata", applicability, skeleton)
        for _, name, applicability, skeleton in METADATA_CONTEXT
    ] + [
        (name, "Form", applicability, skeleton)
        for _, name, applicability, skeleton in FORM_CONTEXT
    ]
    CONTEXT_REGISTRATIONS = "".join(
        f"(unica_metadata::apply_operations::MetadataOperation::{variant})," for variant, _, _, _ in METADATA_CONTEXT
    ) + "".join(
        f"(FormOperation::{variant})," for variant, _, _, _ in FORM_CONTEXT
    )

    MXL_EVENT_ROWS = [
        ("mxl.set", "Mxl", "MxlArea", "Values"),
        ("area.add", "Mxl", "Mxl", "Items"),
        ("event.implement", "Event", "Event", "Target"),
    ]
    MXL_EVENT_REGISTRATIONS = "(MxlOperation::Set), (MxlOperation::AreaAdd), (EventOperation::Implement),"

    DCS_FIXTURE_METADATA = '''dcs_operations!(
        DataSourceAdd => ("dataSource.add", DcsSchema, Items, Vec<DataSourceSpec>),
        QuerySet => ("query.set", DcsQuery, Values, String),
        DataSetRemove => ("dataSet.remove", DcsData, Target, ()),
    );'''
    DCS_FIXTURE_REGISTRATIONS = "(Primitive::DataSourceAdd), (Primitive::QuerySet), (Primitive::DataSetRemove),"
    DCS_FIXTURE_ROWS = [
        ("dataSource.add", "Dcs", "DcsSchema", "Items"),
        ("query.set", "Dcs", "DcsQuery", "Values"),
        ("dataSet.remove", "Dcs", "DcsData", "Target"),
    ]
    PROPERTIES_ROWS = [("props.set", "Properties", "MetadataOrRoleOrSubsystem", "Values")]
    XDTO_ROWS = [
        ("valueType.add", "Xdto", "XdtoPackage", "Values"),
        ("objectType.add", "Xdto", "XdtoPackage", "Values"),
        ("property.add", "Xdto", "XdtoPropertyContainer", "Values"),
        ("type.remove", "Xdto", "XdtoType", "Target"),
        ("property.remove", "Xdto", "XdtoProperty", "Target"),
        ("property.rename", "Xdto", "XdtoProperty", "Values"),
    ]
    XDTO_REGISTRATIONS = "".join(
        f"(XdtoOperation::{variant})," for variant in
        ["AddValueType", "AddObjectType", "AddProperty", "RemoveType", "RemoveProperty", "RenameProperty"]
    )

    def renderer(self):
        module_path = REPO_ROOT / "scripts" / "ci" / "render-acceptance-registry.py"
        spec = importlib.util.spec_from_file_location("render_acceptance_registry", module_path)
        self.assertIsNotNone(spec)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def family_source(self, family):
        return self.renderer().FAMILY_SOURCES[family].read_text(encoding="utf-8")

    def test_registry_document_is_rendered_from_the_corpus(self) -> None:
        with patch("subprocess.Popen", side_effect=AssertionError("registry rendering must not launch Cargo or the product")):
            module = self.renderer()
            rendered = module.render_document()
        self.assertIn("## Покрытие реестра `apply`", rendered)
        corpus = load_acceptance_corpus()
        self.assertIn(f"**{len(corpus['scenarios'])} сценариев**", rendered)
        for scenario in corpus["scenarios"]:
            self.assertIn(f"\n| {scenario['id']} | ", rendered)
        page = module.render_html(rendered)
        self.assertIn('id="S047"', page)
        self.assertIn("описан в <code>tests/fixtures/acceptance/README.md</code>", page)

    def test_role_rows_keep_their_arguments_order_and_implemented_status(self) -> None:
        module = self.renderer()
        registry, implemented = module.load_apply_registry()
        self.assertEqual(
            [entry for entry in registry if entry[1] == "Role"],
            [
                ("role.create", "Role", "Configuration", "Values"),
                ("right.add", "Role", "Role", "Items"),
                ("right.set", "Role", "RoleRight", "Values"),
            ],
        )
        rendered = module.render_document()
        for operation, args in [("role.create", "values"), ("right.add", "items"), ("right.set", "values")]:
            self.assertIn(operation, implemented)
            self.assertIn(f"| `{operation}` | `{args}` | да | ", rendered)

    SOURCE_ROWS = [
        ("code.insert", "Code", "CodeInsert", "Text"),
        ("code.replace", "Code", "Code", "Text"),
        ("subsystem.create", "Subsystem", "Configuration", "Values"),
        ("content.add", "Subsystem", "Subsystem", "Items"),
        ("content.remove", "Subsystem", "Subsystem", "Items"),
        ("childSubsystem.add", "Subsystem", "Subsystem", "Items"),
        ("childSubsystem.remove", "Subsystem", "NestedSubsystem", "Target"),
        ("commandVisibility.set", "Interface", "Interface", "Items"),
        ("commandPlacement.set", "Interface", "Interface", "Items"),
        ("commandOrder.set", "Interface", "InterfaceGroup", "Values"),
        ("groupOrder.set", "Interface", "Interface", "Values"),
        ("subsystemOrder.set", "Interface", "Configuration", "Values"),
    ]

    SOURCE_REGISTRATIONS = """
        (CodeOperation::Insert), (CodeOperation::Replace),
        (SubsystemOperation::Create), (SubsystemOperation::ContentAdd),
        (SubsystemOperation::ContentRemove), (SubsystemOperation::ChildAdd),
        (SubsystemOperation::ChildRemove), (InterfaceOperation::VisibilitySet),
        (InterfaceOperation::PlacementSet), (InterfaceOperation::CommandOrderSet),
        (InterfaceOperation::GroupOrderSet), (InterfaceOperation::SubsystemOrderSet),
    """

    def test_source_operations_fail_closed_when_typed_registration_is_incomplete(self):
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),"
        table = roles + self.SOURCE_REGISTRATIONS
        registry, _ = self.registration_fixture(table, metadata, complete_source=False)
        self.assertEqual(len([entry for entry in registry if entry[1] in {"Code", "Subsystem", "Interface"}]), 12)
        for typed, raw in [
            ("(CodeOperation::Insert)", '("code.insert", Code, CodeInsert, Text)'),
            ("(SubsystemOperation::Create)", '("subsystem.create", Subsystem, Configuration, Values)'),
            ("(InterfaceOperation::VisibilitySet)", '("commandVisibility.set", Interface, Interface, Items)'),
        ]:
            for broken in [table.replace(typed + ",", ""), table + typed + ",", table.replace(typed, raw), table.replace(typed, typed.replace("::", "::Unknown"))]:
                with self.subTest(typed=typed, broken=broken), self.assertRaises(RuntimeError):
                    self.registration_fixture(broken, metadata, complete_source=False)

    def registration_fixture(self, table, metadata, additional_metadata=None, *, complete_support=True, complete_context=True, complete_mxl_event=True, complete_dcs=True, complete_properties=True, complete_xdto=True, complete_source=True):
        # A family-focused fixture keeps unrelated typed families complete.
        module = self.renderer()
        families = dict(module.TYPED_FAMILIES)
        sources = dict(additional_metadata or {})
        sources.setdefault("dcs_operation.rs", self.DCS_FIXTURE_METADATA)
        for family in ["support", "metadata", "form", "mxl", "event", "properties", "xdto", "code", "subsystem", "interface"]:
            filename = f"{family}_operation.rs"
            if filename not in sources:
                sources[filename] = self.family_source(family.title())
        if complete_support:
            table += "\n(SupportOperation::CapabilitySet), (SupportOperation::RuleSet),"
        if complete_context:
            table += "\n" + self.CONTEXT_REGISTRATIONS
        if complete_mxl_event:
            table += "\n" + self.MXL_EVENT_REGISTRATIONS
        if complete_dcs:
            table += "\n" + self.DCS_FIXTURE_REGISTRATIONS
        if complete_properties:
            table += "\n(PropertiesOperation::Set),"
        if complete_xdto:
            table += "\n" + self.XDTO_REGISTRATIONS
        if complete_source:
            table += "\n" + self.SOURCE_REGISTRATIONS
        module = self.renderer()
        with tempfile.TemporaryDirectory(prefix="typed-registry-") as raw:
            root = Path(raw)
            registry = root / "apply.rs"
            registry.write_text(
                "\noperation_descriptors!(\n" + table + "\n);\n"
                'const IMPLEMENTED_APPLY_OPERATIONS: &[&str] = &["role.create", "right.add", "right.set"];\n',
                encoding="utf-8",
            )
            if metadata is not None:
                (root / "role_operation.rs").write_text(metadata, encoding="utf-8")
            for filename, source in sources.items():
                if source is not None:
                    (root / filename).write_text(source, encoding="utf-8")
            paths = {family: root / f"{family.lower()}_operation.rs" for family in families}
            with patch.object(module, "APPLY_REGISTRY", registry), \
                    patch.object(module, "TYPED_FAMILIES", families), \
                    patch.object(module, "FAMILY_SOURCES", paths):
                return module.load_apply_registry()

    def product_registration_fixture(self, source, changed_metadata=None):
        module = self.renderer()
        with tempfile.TemporaryDirectory(prefix="product-registry-") as raw:
            root = Path(raw)
            registry = root / "apply.rs"
            registry.write_text(source, encoding="utf-8")
            paths = dict(module.FAMILY_SOURCES)
            for family, metadata in (changed_metadata or {}).items():
                paths[family] = root / f"{family.lower()}.rs"
                if metadata is not None:
                    paths[family].write_text(metadata, encoding="utf-8")
            with patch.object(module, "APPLY_REGISTRY", registry), \
                    patch.object(module, "FAMILY_SOURCES", paths):
                return module.load_apply_registry()

    def test_all_product_families_read_their_physical_owner_tables(self) -> None:
        module = self.renderer()
        self.assertEqual(set(module.TYPED_FAMILIES), set(module.FAMILY_SOURCES))
        self.assertEqual(len(module.TYPED_FAMILIES), 12)
        registry, _ = module.load_apply_registry()
        expected = {
            "Subsystem": [
                ("subsystem.create", "Subsystem", "Configuration", "Values"),
                ("content.add", "Subsystem", "Subsystem", "Items"),
                ("content.remove", "Subsystem", "Subsystem", "Items"),
                ("childSubsystem.add", "Subsystem", "Subsystem", "Items"),
                ("childSubsystem.remove", "Subsystem", "NestedSubsystem", "Target"),
            ],
            "Interface": [
                ("commandVisibility.set", "Interface", "Interface", "Items"),
                ("commandPlacement.set", "Interface", "Interface", "Items"),
                ("commandOrder.set", "Interface", "InterfaceGroup", "Values"),
                ("groupOrder.set", "Interface", "Interface", "Values"),
                ("subsystemOrder.set", "Interface", "Configuration", "Values"),
            ],
            "Code": [
                ("code.insert", "Code", "CodeInsert", "Text"),
                ("code.replace", "Code", "Code", "Text"),
            ],
        }
        for family, rows in expected.items():
            with self.subTest(family=family):
                self.assertEqual([row for row in registry if row[1] == family], rows)
        self.assertEqual(registry[110:115], expected["Subsystem"])
        self.assertEqual(registry[117:122], expected["Interface"])
        self.assertEqual(registry[122:124], expected["Code"])

    def test_every_registered_apply_operation_has_a_published_integration_case(self) -> None:
        # The inventory is the closed `apply` registry of all typed families
        # against the `source` profile, which `AcceptanceCorpusRunTests`
        # executes in the mandatory queue.  A newly registered operation
        # without a published and checked case fails here, not as a skip.
        with patch("subprocess.Popen", side_effect=AssertionError("the inventory must not launch a process")):
            module = self.renderer()
            registry, _ = module.load_apply_registry()
        self.assertEqual({row[1] for row in registry}, set(module.TYPED_FAMILIES))
        cases = apply_operation_cases(select_profile(load_acceptance_corpus(), "source")["scenarios"], registry)
        missing = [(operation, family) for operation, family, _, _ in registry if (operation, family) not in cases]
        self.assertEqual(
            missing, [],
            "a registered apply operation is not published with checked files by the executed acceptance "
            "corpus; add a scenario under tests/fixtures/acceptance/scenarios/",
        )

    def test_operation_inventory_counts_only_published_apply_plans_of_their_family(self) -> None:
        def preview(expect, at, *operations):
            return {"tool": "unica.apply", "expect": expect, "args": {"at": at, "ops": [{"op": name} for name in operations]}}

        pinned = {"path": "src/X.xml", "kind": "xml", "select": "./{*}Name", "eq": ["X"]}
        vague = [{"path": "src/X.xml", "kind": "exists", "eq": True},
                 {"path": "src/X.xml", "kind": "sha256", "notEq": "$capture.before"},
                 {"path": "src/X.xml", "kind": "sha256", "eq": "$capture.before"}]

        def publish(expect=("ok",), checked=True, token="$executionToken", assertions=(pinned,)):
            step = {"tool": "unica.apply", "expect": list(expect), "args": {"executionToken": token}}
            if checked:
                step["fileAssertions"] = list(assertions)
            return step

        registry = [
            ("props.set", "Properties", "Metadata", "Values"),
            ("field.add", "Dcs", "DcsField", "Values"),
            ("item.remove", "Form", "FormItem", "Target"),
            ("attribute.add", "Metadata", "MetadataAttributeContainer", "Items"),
            ("attribute.add", "Form", "Form", "Items"),
        ]
        catalog, form = "main:Catalog.X", "main:Catalog.X.Form.Y"
        scenarios = [
            {"id": "A", "wire": [preview(["ok"], catalog, "props.set", "attribute.add"),
                                 preview(["refused"], catalog, "field.add"), publish()]},
            {"id": "B", "wire": [
                preview(["ok"], catalog, "field.add"),
                preview(["ok"], form, "attribute.add"), publish(),
                publish(),
                preview(["ok"], form, "item.remove"), publish(checked=False),
                preview(["ok"], form, "item.remove"), publish(token="literal"),
                preview(["ok"], form, "item.remove"), publish(assertions=vague),
            ]},
            {"id": "H", "wire": [
                preview(["ok"], catalog, "field.add"), publish(expect=("refused",)),
                publish(assertions=[{"path": "src/X.xml", "kind": "exists", "eq": False}]),
            ]},
            {"id": "G", "wire": [
                preview(["ok", "task"], catalog, "props.set"), publish(),
                preview(["gap"], catalog, "props.set"), publish(),
                {"tool": "unica.view", "expect": ["ok"], "args": {"at": catalog, "ops": [{"op": "props.set"}]}},
                publish(),
            ]},
            {"id": "C", "wire": [{"tool": "unica.apply", "expect": ["ok"], "args": {
                "at": catalog, "ops": [{"op": "attribute.add", "args": {"at": form}}, {"op": "unregistered.add"}]}},
                publish()]},
            {"id": "D", "wire": [preview(["ok"], catalog, "props.set")]},
            {"id": "F", "wire": [
                {**preview(["ok"], form, "item.remove"), "captures": {"remove": {"pointer": "/data/executionToken"}}},
                preview(["ok"], catalog, "field.add"),
                publish(token="$capture.remove"),
            ]},
        ]
        self.assertEqual(apply_operation_cases(scenarios, registry), {
            ("props.set", "Properties"): ["A"],
            ("attribute.add", "Metadata"): ["A"],
            ("attribute.add", "Form"): ["B", "C"],
            ("item.remove", "Form"): ["F"],
            ("field.add", "Dcs"): ["H"],
        })
        self.assertTrue(pins_published_file({"path": "a", "kind": "sha256", "eq": "0" * 64}))
        self.assertTrue(names_form("main:CommonForm.Main"))
        self.assertIsNone(names_form("main:Catalog.X.Form.Y.Attribute.Z"))
        self.assertIs(names_form("main:Configuration"), False)
        nested = [{"id": "E", "wire": [preview(["ok"], "main:Catalog.X.Form.Y.Attribute.Z", "attribute.add")]}]
        with self.assertRaises(ValueError):
            apply_operation_cases(nested, registry)
        with self.assertRaises(ValueError):
            apply_operation_cases(scenarios, registry + [("attribute.add", "Xdto", "XdtoType", "Items")])

    def test_metadata_registration_requires_the_actual_qualified_owner(self) -> None:
        with patch("subprocess.Popen", side_effect=AssertionError("registry inspection must not launch a process")):
            module = self.renderer()
            operation = "unica_metadata::apply_operations::MetadataOperation"
            self.assertEqual(module.TYPED_FAMILIES["Metadata"], operation)
            original = module.APPLY_REGISTRY.read_text(encoding="utf-8")
            registry, _ = module.load_apply_registry()
            self.assertEqual(
                [row for row in registry if row[1] == "Metadata"],
                [row for row in self.CONTEXT_ROWS if row[1] == "Metadata"],
            )
            self.assertEqual(len(registry), 125)
            self.assertEqual(len({row[0] for row in registry}), 122)
            registration = f"({operation}::ObjectBorrow),"
            self.assertEqual(original.count(registration), 1)
            replacements = {
                "old bare alias": "(MetadataOperation::ObjectBorrow),",
                "wrong crate same enum suffix": "(other_metadata::apply_operations::MetadataOperation::ObjectBorrow),",
                "wrong module same enum suffix": "(unica_metadata::request::MetadataOperation::ObjectBorrow),",
                "unknown prefix": f"(unknown::{operation}::ObjectBorrow),",
                "unknown variant": f"({operation}::Unknown),",
                "generic path": f"({operation}::<Other>::ObjectBorrow),",
                "missing": "",
                "duplicate": registration * 2,
                "raw replacement": '("object.borrow", Metadata, ExtensionRoot, From),',
                "unconsumed entry": registration + "unexpected_registration!(),",
            }
            for case, replacement in replacements.items():
                changed = original.replace(registration, replacement, 1)
                self.assertNotEqual(changed, original, f"{case} must alter a real registration")
                with self.subTest(case=case), self.assertRaises(RuntimeError):
                    self.product_registration_fixture(changed)

    def test_code_subsystem_and_interface_tables_fail_closed_in_product_composition(self) -> None:
        module = self.renderer()
        original = module.APPLY_REGISTRY.read_text(encoding="utf-8")
        for family in ["Code", "Subsystem", "Interface"]:
            operation = module.TYPED_FAMILIES[family]
            metadata = self.family_source(family)
            table_name = f"{family.lower()}_operations"
            entries = module._registration_metadata(family)
            for variant, (name, _, applicability, skeleton) in entries.items():
                registration = f"({operation}::{variant}),"
                row = f'{variant} => ("{name}", {applicability}, {skeleton})'
                raw = f'("{name}", {family}, {applicability}, {skeleton}),'
                self.assertEqual(original.count(registration), 1)
                self.assertEqual(metadata.count(row), 1)
                cases = {
                    "missing registration": (original.replace(registration, ""), metadata),
                    "duplicate registration": (original.replace(registration, registration * 2), metadata),
                    "unknown variant": (original.replace(registration, f"({operation}::Unknown),"), metadata),
                    "wrong family": (original.replace(registration, f"(OtherOperation::{variant}),"), metadata),
                    "raw replacement": (original.replace(registration, raw), metadata),
                    "raw beside": (original.replace(registration, registration + raw), metadata),
                    "raw future name": (original.replace(registration, registration + f'("future.operation", {family}, {applicability}, {skeleton}),'), metadata),
                    "metadata missing": (original, None),
                    "metadata empty": (original, f"{table_name}!();"),
                    "metadata row missing": (original, metadata.replace(row + ",", "")),
                    "metadata incomplete": (original, metadata.replace(row, f'{variant} => ("{name}", {applicability})')),
                    "metadata nonliteral": (original, metadata.replace(row, f'{variant} => (NAME, {applicability}, {skeleton})')),
                    "metadata repeated": (original, metadata + f'\n{table_name}!(Probe => ("future.operation", {applicability}, {skeleton}),);'),
                }
                for case, (source, changed) in cases.items():
                    with self.subTest(family=family, variant=variant, case=case), self.assertRaises(RuntimeError):
                        self.product_registration_fixture(source, {family: changed})

    def test_xdto_rows_have_complete_typed_metadata_and_preserve_registry_order(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        metadata = (REPO_ROOT / "crates/unica-xdto/src/operations.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),"
        table = roles + self.XDTO_REGISTRATIONS
        registry, implemented = self.renderer().load_apply_registry()
        self.assertEqual([entry for entry in registry if entry[1] == "Xdto"], self.XDTO_ROWS)
        self.assertEqual(registry[104:110], self.XDTO_ROWS)
        self.assertLessEqual({entry[0] for entry in self.XDTO_ROWS}, implemented)
        row = 'AddValueType => ("valueType.add", XdtoPackage, Values)'
        self.assertIn(row, metadata)
        cases = {
            "missing registration": (table.replace("(XdtoOperation::AddProperty),", ""), metadata),
            "duplicate registration": (table + "(XdtoOperation::AddProperty),", metadata),
            "unknown variant": (table.replace("XdtoOperation::AddProperty", "XdtoOperation::Unknown"), metadata),
            "raw replacement": (table.replace("(XdtoOperation::AddValueType),", '("valueType.add", Xdto, XdtoPackage, Values),'), metadata),
            "raw unknown name": (table + '("future.operation", Xdto, XdtoPackage, Values),', metadata),
            "missing metadata": (table, None),
            "empty metadata": (table, "xdto_operations!();"),
            "missing metadata row": (table, metadata.replace(row + ",", "")),
            "nonliteral name": (table, metadata.replace(row, 'AddValueType => (NAME, XdtoPackage, Values)')),
        }
        for label, (source, changed_metadata) in cases.items():
            with self.subTest(label=label), self.assertRaises(RuntimeError):
                self.registration_fixture(source, role_source, {"xdto_operation.rs": changed_metadata}, complete_xdto=False)

    def test_properties_rows_use_complete_typed_metadata_and_fail_closed(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        metadata = (REPO_ROOT / "crates/unica-metadata/src/descriptor_operations.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),"
        registration = "(PropertiesOperation::Set),"
        table = roles + registration
        row = 'Set => ("props.set", MetadataOrRoleOrSubsystem, Values)'
        raw = '("props.set", Properties, MetadataOrRoleOrSubsystem, Values),'
        actual, _ = self.registration_fixture(table, role_source, complete_properties=False)
        self.assertEqual([entry for entry in actual if entry[1] == "Properties"],
                         [("props.set", "Properties", "MetadataOrRoleOrSubsystem", "Values")])
        cases = {
            "missing registration": (roles, metadata),
            "duplicate registration": (table + registration, metadata),
            "unknown variant": (table.replace("PropertiesOperation::Set", "PropertiesOperation::Unknown"), metadata),
            "wrong family": (table.replace("PropertiesOperation::Set", "RoleOperation::Set"), metadata),
            "raw replacement": (roles + raw, metadata),
            "raw beside complete typed": (table + raw, metadata),
            "raw unknown name": (table + '("future.operation", Properties, Form, Items),', metadata),
            "raw after literal rename": (table + raw, metadata.replace('"props.set"', '"props.probe"')),
            "raw renamed name": (table + '("props.probe", Properties, Form, Items),', metadata.replace('"props.set"', '"props.probe"')),
            "missing metadata": (table, None),
            "empty metadata": (table, "properties_operations!();"),
            "missing metadata row": (table, metadata.replace(row + ",", "")),
            "incomplete metadata": (table, metadata.replace(row, 'Set => ("props.set", MetadataOrRoleOrSubsystem)')),
            "nonliteral name": (table, metadata.replace(row, 'Set => (NAME, MetadataOrRoleOrSubsystem, Values)')),
            "duplicate metadata": (table, metadata + '\nproperties_operations!(Probe => ("props.probe", Form, Items),);'),
        }
        for label, (source, changed_metadata) in cases.items():
            with self.subTest(label=label), self.assertRaises(RuntimeError):
                self.registration_fixture(source, role_source, {"properties_operation.rs": changed_metadata}, complete_properties=False)
        registry, implemented = self.renderer().load_apply_registry()
        self.assertEqual([entry for entry in registry if entry[1] == "Properties"],
                         [("props.set", "Properties", "MetadataOrRoleOrSubsystem", "Values")])
        self.assertIn("props.set", implemented)

    def test_typed_support_rows_keep_their_metadata_and_registry_order(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        support_metadata = '''support_operations!(
            CapabilitySet => ("supportCapability.set", Configuration, Values),
            RuleSet => ("supportRule.set", SupportRule, Values),
        );'''
        registry, _ = self.registration_fixture(
            '(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n'
            '(SupportOperation::CapabilitySet),\n'
            '("probe.add", Probe, Subsystem, Items),\n'
            '(SupportOperation::RuleSet),',
            metadata,
            {"support_operation.rs": support_metadata},
            complete_support=False,
        )
        self.assertEqual(registry, [
            ("role.create", "Role", "Configuration", "Values"),
            ("right.add", "Role", "Role", "Items"),
            ("right.set", "Role", "RoleRight", "Values"),
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("probe.add", "Probe", "Subsystem", "Items"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ] + self.CONTEXT_ROWS + self.MXL_EVENT_ROWS + self.DCS_FIXTURE_ROWS + self.PROPERTIES_ROWS + self.XDTO_ROWS + self.SOURCE_ROWS)

    def test_support_rows_keep_their_arguments_order_and_implemented_status(self) -> None:
        module = self.renderer()
        registry, implemented = module.load_apply_registry()
        self.assertEqual(
            [entry for entry in registry if entry[1] == "Support"],
            [
                ("supportCapability.set", "Support", "Configuration", "Values"),
                ("supportRule.set", "Support", "SupportRule", "Values"),
            ],
        )
        rendered = module.render_document()
        for operation in ["supportCapability.set", "supportRule.set"]:
            self.assertIn(operation, implemented)
            self.assertIn(f"| `{operation}` | `values` | да | ", rendered)

    def test_typed_rows_preserve_colliding_names_and_raw_neighbors(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        registry, _ = self.registration_fixture(
            '(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n'
            '(unica_metadata::apply_operations::MetadataOperation::AttributeAdd),\n'
            '(unica_metadata::apply_operations::MetadataOperation::AttributeSet),\n'
            '(unica_metadata::apply_operations::MetadataOperation::ObjectCreate),\n'
            '("probe.add", Probe, Subsystem, Items),\n'
            '(FormOperation::Add), (FormOperation::Set), (FormOperation::Remove),\n'
            '(FormOperation::AttributeAdd),\n'
            '(unica_metadata::apply_operations::MetadataOperation::AttributeRemove),\n'
            '(unica_metadata::apply_operations::MetadataOperation::CommandAdd),\n'
            '(unica_metadata::apply_operations::MetadataOperation::CommandSet), (unica_metadata::apply_operations::MetadataOperation::CommandRemove),\n'
            '(FormOperation::ItemAdd),\n'
            '(FormOperation::CommandAdd),\n'
            '(FormOperation::ItemRemove), (FormOperation::ItemMove), (FormOperation::EventBind),\n'
            '(FormOperation::PropsSet),',
            metadata,
            {
                "metadata_operation.rs": '''metadata_operations!(
                    ObjectCreate => ("object.create", Configuration, Values),
                    AttributeAdd => ("attribute.add", MetadataAttributeContainer, Items),
                    AttributeSet => ("attribute.set", MetadataAttribute, Values),
                    AttributeRemove => ("attribute.remove", MetadataAttribute, Target),
                    CommandAdd => ("command.add", MetadataCommands, Items),
                    CommandSet => ("command.set", MetadataCommand, Values),
                    CommandRemove => ("command.remove", MetadataCommand, Target),
                );''',
            },
            complete_context=False,
        )
        self.assertEqual(registry, [
            ("role.create", "Role", "Configuration", "Values"),
            ("right.add", "Role", "Role", "Items"),
            ("right.set", "Role", "RoleRight", "Values"),
            ("attribute.add", "Metadata", "MetadataAttributeContainer", "Items"),
            ("attribute.set", "Metadata", "MetadataAttribute", "Values"),
            ("object.create", "Metadata", "Configuration", "Values"),
            ("probe.add", "Probe", "Subsystem", "Items"),
            ("form.add", "Form", "MetadataForms", "Items"),
            ("form.set", "Form", "MetadataForms", "Values"),
            ("form.remove", "Form", "Form", "Target"),
            ("attribute.add", "Form", "Form", "Items"),
            ("attribute.remove", "Metadata", "MetadataAttribute", "Target"),
            ("command.add", "Metadata", "MetadataCommands", "Items"),
            ("command.set", "Metadata", "MetadataCommand", "Values"),
            ("command.remove", "Metadata", "MetadataCommand", "Target"),
            ("item.add", "Form", "FormItemContainer", "Items"),
            ("command.add", "Form", "Form", "Items"),
            ("item.remove", "Form", "FormItem", "Target"),
            ("item.move", "Form", "FormItem", "Values"),
            ("event.bind", "Form", "FormEvent", "Values"),
            ("props.set", "Form", "Form", "Values"),
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ] + self.MXL_EVENT_ROWS + self.DCS_FIXTURE_ROWS + self.PROPERTIES_ROWS + self.XDTO_ROWS + self.SOURCE_ROWS)

    def test_fully_typed_metadata_and_form_keep_registry_rows(self) -> None:
        module = self.renderer()
        registry, implemented = module.load_apply_registry()
        for family, migrated_names, remaining in [
            ("Metadata", {name for _, name, _, _ in self.METADATA_CONTEXT}, 0),
            ("Form", {name for _, name, _, _ in self.FORM_CONTEXT}, 0),
        ]:
            self.assertEqual(
                [entry for entry in registry if entry[1] == family and entry[0] in migrated_names],
                [entry for entry in self.CONTEXT_ROWS if entry[1] == family],
            )
            self.assertEqual(sum(entry[1] == family and entry[0] not in migrated_names for entry in registry), remaining)
            self.assertLessEqual(migrated_names, implemented)
        self.assertIn(("content.set", "Metadata", "ExchangePlan", "Items"), registry)
        self.assertIn("content.set", implemented)
        self.assertEqual(len(registry), 125)
        self.assertEqual(len({entry[0] for entry in registry}), 122)

    def test_dcs_rows_keep_all_53_metadata_and_original_registry_positions(self) -> None:
        module = self.renderer()
        registry, implemented = module.load_apply_registry()
        expected = [
            (name, "Dcs", applicability, skeleton)
            for name, applicability, skeleton in [
                ("dataSource.add", "DcsSchema", "Items"),
                ("dataSource.set", "DcsDataSource", "Values"),
                ("dataSource.remove", "DcsDataSource", "Target"),
                ("dataSet.add", "DcsDataContainer", "Items"),
                ("dataSet.set", "DcsData", "Values"),
                ("dataSet.remove", "DcsData", "Target"),
                ("field.add", "DcsData", "Items"),
                ("field.set", "DcsField", "Values"),
                ("field.remove", "DcsField", "Target"),
                ("fieldRole.set", "DcsField", "Values"),
                ("parameter.add", "DcsSchema", "Items"),
                ("parameter.set", "DcsParameter", "Values"),
                ("parameter.remove", "DcsParameter", "Target"),
                ("parameter.rename", "DcsParameter", "Values"),
                ("parameter.reorder", "DcsSchema", "Items"),
                ("calculatedField.add", "DcsSchema", "Items"),
                ("calculatedField.remove", "DcsCalculatedField", "Target"),
                ("total.add", "DcsSchema", "Items"),
                ("total.remove", "DcsTotal", "Target"),
                ("variant.add", "DcsSchema", "Items"),
                ("variant.set", "DcsSettings", "Values"),
                ("variant.remove", "DcsSettings", "Target"),
                ("query.set", "DcsQuery", "Values"),
                ("query.patch", "DcsQuery", "Values"),
                ("filter.add", "DcsSettingsContainer", "Items"),
                ("filter.set", "DcsSettingsContainer", "Values"),
                ("filter.remove", "DcsSettingsContainer", "Values"),
                ("filter.clear", "DcsSettingsContainer", "Values"),
                ("selection.add", "DcsSettingsContainer", "Items"),
                ("selection.clear", "DcsSettingsContainer", "Values"),
                ("order.add", "DcsSettingsContainer", "Items"),
                ("order.clear", "DcsSettingsContainer", "Values"),
                ("dataParameter.add", "DcsSettings", "Items"),
                ("dataParameter.set", "DcsSettings", "Values"),
                ("outputParameter.set", "DcsSettingsContainer", "Values"),
                ("conditionalAppearance.add", "DcsSettingsContainer", "Items"),
                ("conditionalAppearance.clear", "DcsSettingsContainer", "Values"),
                ("structure.add", "DcsSettingsContainer", "Items"),
                ("structure.set", "DcsSettings", "Values"),
                ("structure.patch", "DcsStructureItem", "Values"),
                ("structure.remove", "DcsStructureItem", "Target"),
                ("dataSetLink.add", "DcsSchema", "Items"),
                ("calculatedField.set", "DcsCalculatedField", "Values"),
                ("total.set", "DcsTotal", "Values"),
                ("selection.set", "DcsSettingsContainer", "Values"),
                ("selection.remove", "DcsSettingsContainer", "Values"),
                ("order.set", "DcsSettingsContainer", "Values"),
                ("order.remove", "DcsSettingsContainer", "Values"),
                ("dataParameter.remove", "DcsSettings", "Values"),
                ("outputParameter.remove", "DcsSettingsContainer", "Values"),
                ("conditionalAppearance.remove", "DcsSettingsContainer", "Values"),
                ("dataSetLink.set", "DcsSchema", "Values"),
                ("dataSetLink.remove", "DcsSchema", "Values"),
            ]
        ]
        self.assertEqual(registry[49:102], expected)
        self.assertEqual([row for row in registry if row[1] == "Dcs"], expected)
        self.assertEqual(registry[48][0], "right.set")
        self.assertEqual(registry[102][0], "mxl.set")
        self.assertLessEqual({row[0] for row in expected}, implemented)

    def test_typed_dcs_rows_keep_payload_metadata_and_raw_registry_neighbors(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        registry, _ = self.registration_fixture(
            '(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n'
            '(Primitive::QuerySet), ("probe.add", Probe, Subsystem, Items),\n'
            '(Primitive::DataSetRemove), ("probe.replace", Probe, Code, Text),\n'
            '(Primitive::DataSourceAdd),',
            role_source, complete_dcs=False,
        )
        self.assertEqual(registry, [
            ("role.create", "Role", "Configuration", "Values"),
            ("right.add", "Role", "Role", "Items"),
            ("right.set", "Role", "RoleRight", "Values"),
            ("query.set", "Dcs", "DcsQuery", "Values"),
            ("probe.add", "Probe", "Subsystem", "Items"),
            ("dataSet.remove", "Dcs", "DcsData", "Target"),
            ("probe.replace", "Probe", "Code", "Text"),
            ("dataSource.add", "Dcs", "DcsSchema", "Items"),
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ] + self.CONTEXT_ROWS + self.MXL_EVENT_ROWS + self.PROPERTIES_ROWS + self.XDTO_ROWS + self.SOURCE_ROWS)

    def test_dcs_metadata_and_registrations_are_complete_and_fail_closed(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n"
        table = roles + self.DCS_FIXTURE_REGISTRATIONS
        metadata = self.DCS_FIXTURE_METADATA
        self.registration_fixture(table, role_source, complete_dcs=False)
        row = 'QuerySet => ("query.set", DcsQuery, Values, String)'
        cases = {
            "unknown typed family": (table.replace("Primitive::QuerySet", "DcsOperation::QuerySet"), metadata),
            "unknown variant": (table.replace("Primitive::QuerySet", "Primitive::Unknown"), metadata),
            "wrong typed family": (table.replace("Primitive::QuerySet", "unica_metadata::apply_operations::MetadataOperation::QuerySet"), metadata),
            "all family registrations missing": (roles, metadata),
            "raw unknown name": (table + '("future.operation", Dcs, DcsSchema, Items),', metadata),
            "raw original name after rename": (
                table + '("query.set", Dcs, DcsQuery, Values),', metadata.replace('"query.set"', '"query.probe"'),
            ),
            "raw renamed family name": (
                table + '("query.probe", Dcs, DcsQuery, Values),', metadata.replace('"query.set"', '"query.probe"'),
            ),
            "unparsed registration": (table + "unexpected_registration!(),", metadata),
            "missing metadata file": (table, None),
            "missing metadata table": (table, metadata.replace("dcs_operations!", "previous_operations!")),
            "repeated metadata table": (table, metadata + '\ndcs_operations!(Extra => ("extra.add", DcsSchema, Items, String),);'),
            "empty metadata table": (table, "dcs_operations!();"),
            "unterminated metadata table": (table, 'dcs_operations!(QuerySet => ("query.set", DcsQuery, Values, String),'),
            "missing metadata row": (table, metadata.replace(row + ",", "")),
            "unsupported metadata row": (table, metadata.replace(row, "QuerySet => operation_metadata()")),
            "unparsed trailing metadata": (table, metadata.replace(row + ",", row + ", unexpected!(),")),
            "nonliteral metadata name": (table, metadata.replace(row, 'QuerySet => (QUERY_SET, DcsQuery, Values, String)')),
            "duplicate metadata variant": (table, metadata.replace("QuerySet =>", "DataSourceAdd =>")),
            "duplicate metadata name": (table, metadata.replace('"query.set"', '"dataSource.add"')),
        }
        for variant, (name, family, applicability, skeleton) in zip(
            ["DataSourceAdd", "QuerySet", "DataSetRemove"], self.DCS_FIXTURE_ROWS, strict=True,
        ):
            registration = f"(Primitive::{variant}),"
            raw = f'("{name}", {family}, {applicability}, {skeleton}),'
            cases[f"missing {name}"] = (table.replace(registration, ""), metadata)
            cases[f"duplicate {name}"] = (table + registration, metadata)
            cases[f"raw {name} replacement"] = (table.replace(registration, raw), metadata)
            cases[f"raw {name} beside complete typed"] = (table + raw, metadata)
        for case, (source, changed_metadata) in cases.items():
            with self.subTest(case=case), self.assertRaises(RuntimeError):
                self.registration_fixture(
                    source, role_source, {"dcs_operation.rs": changed_metadata}, complete_dcs=False,
                )

    def test_dcs_payload_type_syntax_is_consumed_without_skipping_metadata(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),"
        for payload in ["DataSourceSpec", "LocalizedText", "Vec<String>", "Vec < DataSourceSpec >", "()", "(\n )"]:
            changed = self.DCS_FIXTURE_METADATA.replace("Vec<DataSourceSpec>", payload)
            with self.subTest(payload=payload):
                registry, _ = self.registration_fixture(roles, role_source, {"dcs_operation.rs": changed})
                self.assertEqual([row for row in registry if row[1] == "Dcs"], self.DCS_FIXTURE_ROWS)
        for payload in ["", "Vec<>", "Vec<DataSourceSpec", "Vec<DataSourceSpec>>", "Vec<String, Other>", "payload()", "String, Extra", "("]:
            changed = self.DCS_FIXTURE_METADATA.replace("Vec<DataSourceSpec>", payload)
            with self.subTest(payload=payload), self.assertRaises(RuntimeError):
                self.registration_fixture(roles, role_source, {"dcs_operation.rs": changed})
        without_payload = self.DCS_FIXTURE_METADATA.replace(
            "Items, Vec<DataSourceSpec>", "Items",
        )
        with self.assertRaises(RuntimeError):
            self.registration_fixture(roles, role_source, {"dcs_operation.rs": without_payload})

    def test_contextual_typed_metadata_cannot_be_missing_duplicated_or_registered_as_raw(self) -> None:
        role_metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n"
        table = roles + self.CONTEXT_REGISTRATIONS
        self.registration_fixture(table, role_metadata, complete_context=False)
        for family, applicability in [("Metadata", "MetadataCommands"), ("Form", "Form")]:
            filename = f"{family.lower()}_operation.rs"
            table_name = f"{family.lower()}_operations"
            operation_type = self.renderer().TYPED_FAMILIES[family]
            metadata = self.family_source(family)
            last_row = f'    CommandAdd => ("command.add", {applicability}, Items),\n'
            self.assertIn(last_row, metadata)
            raw = f'("command.add", {family}, {applicability}, Items),'
            other_type = self.renderer().TYPED_FAMILIES["Form" if family == "Metadata" else "Metadata"]
            operations = self.METADATA_CONTEXT if family == "Metadata" else self.FORM_CONTEXT
            without_family = table
            for variant, _, _, _ in operations:
                without_family = without_family.replace(f"({operation_type}::{variant}),", "")
            cases = {
                "unknown typed family": (table.replace(f"{operation_type}::AttributeAdd", "OtherOperation::AttributeAdd"), metadata),
                "unknown variant": (table.replace(f"{operation_type}::AttributeAdd", f"{operation_type}::Missing"), metadata),
                "wrong typed family": (table.replace(f"{operation_type}::AttributeAdd", f"{other_type}::AttributeAdd"), metadata),
                "missing registration": (table.replace(f"({operation_type}::CommandAdd),", ""), metadata),
                "all family registrations missing": (without_family, metadata),
                "both contextual families missing": (roles, metadata),
                "duplicate registration": (table + f"({operation_type}::CommandAdd),", metadata),
                "raw instead of typed": (table.replace(f"({operation_type}::CommandAdd),", raw), metadata),
                "raw beside complete typed": (table + raw, metadata),
                "missing metadata file": (table, None),
                "missing metadata table": (table, metadata.replace(f"\n{table_name}!(", "\nprevious_operations!(")),
                "repeated metadata table": (table, metadata + f'\n{table_name}!(Extra => ("extra.add", Form, Items),);\n'),
                "empty metadata table": (table, f"{table_name}!();"),
                "unterminated metadata table": (table, f'{table_name}!(CommandAdd => ("command.add", {applicability}, Items),'),
                "missing metadata row": (table, metadata.replace(last_row, "")),
                "incomplete metadata row": (table, metadata.replace(last_row, f'    CommandAdd => ("command.add", {applicability}),\n')),
                "unsupported metadata row": (table, metadata.replace(last_row, "    CommandAdd => operation_metadata(),\n")),
                "nonliteral metadata name": (table, metadata.replace(last_row, f"    CommandAdd => (COMMAND_ADD, {applicability}, Items),\n")),
                "duplicate metadata variant": (table, metadata.replace(last_row, f'    AttributeAdd => ("command.add", {applicability}, Items),\n')),
                "duplicate metadata name": (table, metadata.replace(last_row, f'    CommandAdd => ("attribute.add", {applicability}, Items),\n')),
            }
            for variant, name, applicability, skeleton in operations:
                registration = f"({operation_type}::{variant}),"
                raw_operation = f'("{name}", {family}, {applicability}, {skeleton}),'
                cases[f"wrong {name} typed family"] = (
                    table.replace(registration, f"({other_type}::{variant}),"), metadata,
                )
                if variant == "CommandAdd":
                    continue  # The common cases above already mutate this variant.
                cases[f"missing {name} registration"] = (table.replace(registration, ""), metadata)
                cases[f"duplicate {name} registration"] = (table + registration, metadata)
                cases[f"raw {name} instead of typed"] = (table.replace(registration, raw_operation), metadata)
                cases[f"raw {name} beside complete typed"] = (table + raw_operation, metadata)
            renamed_metadata = metadata.replace('"command.add"', '"command.probe"')
            cases["raw unknown family name"] = (table + f'("future.operation", {family}, Form, Items),', metadata)
            cases["raw original name after literal rename"] = (table + raw, renamed_metadata)
            cases["raw renamed family name"] = (table + f'("command.probe", {family}, Form, Items),', renamed_metadata)
            for case, (source, family_metadata) in cases.items():
                with self.subTest(family=family, case=case), self.assertRaises(RuntimeError):
                    self.registration_fixture(
                        source, role_metadata, {filename: family_metadata}, complete_context=False,
                    )

    def test_mxl_and_event_rows_are_complete_and_fail_closed(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),"
        table = roles + self.MXL_EVENT_REGISTRATIONS
        actual, _ = self.registration_fixture(table, role_source, complete_mxl_event=False)
        self.assertEqual([row for row in actual if row[1] in {"Mxl", "Event"}], self.MXL_EVENT_ROWS)
        for family, variants in [
            ("Mxl", [("Set", "mxl.set", "MxlArea", "Values"), ("AreaAdd", "area.add", "Mxl", "Items")]),
            ("Event", [("Implement", "event.implement", "Event", "Target")]),
        ]:
            filename = f"{family.lower()}_operation.rs"
            metadata = self.family_source(family)
            for variant, name, applicability, skeleton in variants:
                registration = f"({family}Operation::{variant}),"
                row = f'{variant} => ("{name}", {applicability}, {skeleton})'
                raw = f'("{name}", {family}, {applicability}, {skeleton}),'
                cases = {
                    "missing": (table.replace(registration, ""), metadata),
                    "duplicate": (table + registration, metadata),
                    "wrong family": (table.replace(registration, f"(FormOperation::{variant}),"), metadata),
                    "unknown variant": (table.replace(registration, f"({family}Operation::Unknown),"), metadata),
                    "raw replacement": (table.replace(registration, raw), metadata),
                    "raw beside": (table + raw, metadata),
                    "raw unknown": (table + f'("future.operation", {family}, Form, Items),', metadata),
                    "raw renamed bypass": (table + raw, metadata.replace(f'"{name}"', '"future.operation"')),
                    "raw new spelling": (table + f'("future.operation", {family}, Form, Items),', metadata.replace(f'"{name}"', '"future.operation"')),
                    "metadata missing": (table, None),
                    "metadata empty": (table, f"{family.lower()}_operations!();"),
                    "metadata row missing": (table, metadata.replace(row + ",", "")),
                    "metadata incomplete": (table, metadata.replace(row, f'{variant} => ("{name}", {applicability})')),
                    "metadata nonliteral": (table, metadata.replace(row, f'{variant} => (NAME, {applicability}, {skeleton})')),
                    "metadata repeated": (table, metadata + f'\n{family.lower()}_operations!(Probe => ("future.operation", Form, Items),);'),
                }
                for case, (source, changed_metadata) in cases.items():
                    with self.subTest(family=family, variant=variant, case=case), self.assertRaises(RuntimeError):
                        self.registration_fixture(source, role_source, {filename: changed_metadata}, complete_mxl_event=False)
        registry, _ = self.renderer().load_apply_registry()
        self.assertEqual(registry[102:104], self.MXL_EVENT_ROWS[:2])
        self.assertEqual(registry[124], self.MXL_EVENT_ROWS[2])
        self.assertEqual(registry[101][0], "dataSetLink.remove")
        self.assertEqual(registry[104][0], "valueType.add")
        self.assertEqual(registry[123][0], "code.replace")

    def test_typed_role_rows_stay_between_their_raw_registry_neighbors(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        registry, _ = self.registration_fixture(
            '("probe.add", Probe, Subsystem, Items),\n'
            '(RoleOperation::Create),\n'
            '(RoleOperation::RightAdd),\n'
            '// A comment is not a descriptor: (OtherOperation::Hidden),\n'
            '(RoleOperation::RightSet),\n'
            '("probe.replace", Probe, Code, Text),',
            metadata,
        )
        self.assertEqual(registry, [
            ("probe.add", "Probe", "Subsystem", "Items"),
            ("role.create", "Role", "Configuration", "Values"),
            ("right.add", "Role", "Role", "Items"),
            ("right.set", "Role", "RoleRight", "Values"),
            ("probe.replace", "Probe", "Code", "Text"),
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ] + self.CONTEXT_ROWS + self.MXL_EVENT_ROWS + self.DCS_FIXTURE_ROWS + self.PROPERTIES_ROWS + self.XDTO_ROWS + self.SOURCE_ROWS)

    def test_role_metadata_survives_split_impls_and_unrelated_helpers(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        contract = "    pub fn contract(self) -> OperationContract {"
        self.assertEqual(metadata.count(contract), 1)
        metadata = metadata.replace(
            contract,
            '}\n\n// role_operations!(Commented => ("ignored.operation", Configuration, Values),);\n'
            'impl RoleOperation {\n'
            '    #[allow(dead_code)]\n'
            '    fn unrelated_metadata_hint(self) -> &\'static str { "role_operations!(NotATable)" }\n\n'
            + contract,
        )
        registry, _ = self.registration_fixture(
            "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),",
            metadata,
        )
        self.assertEqual(registry, [
            ("role.create", "Role", "Configuration", "Values"),
            ("right.add", "Role", "Role", "Items"),
            ("right.set", "Role", "RoleRight", "Values"),
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ] + self.CONTEXT_ROWS + self.MXL_EVENT_ROWS + self.DCS_FIXTURE_ROWS + self.PROPERTIES_ROWS + self.XDTO_ROWS + self.SOURCE_ROWS)

    def test_unknown_or_incomplete_typed_metadata_cannot_silently_drop_a_role(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        table = '("probe.add", Probe, Subsystem, Items),\n' + "\n".join(
            f"(RoleOperation::{variant})," for variant in ["Create", "RightAdd", "RightSet"]
        )
        last_row = '    RightSet => ("right.set", RoleRight, Values),\n'
        self.assertIn(last_row, metadata)
        cases = {
            "unknown typed family": (table.replace("RoleOperation::Create", "OtherOperation::Create"), metadata),
            "unknown variant": (table.replace("RoleOperation::Create", "RoleOperation::Missing"), metadata),
            "missing registration": (table.replace("(RoleOperation::RightSet),", ""), metadata),
            "all typed registrations missing": ('("probe.add", Probe, Subsystem, Items),', metadata),
            "duplicate registration": (table + "\n(RoleOperation::Create),", metadata),
            "missing metadata file": (table, None),
            "missing metadata table": (table, metadata.replace("\nrole_operations!(", "\nprevious_role_operations!(")),
            "repeated metadata table": (table, metadata + '\nrole_operations!(Extra => ("role.extra", Configuration, Values),);\n'),
            "empty metadata table": (table, "role_operations!();"),
            "unterminated metadata table": (table, 'role_operations!(Create => ("role.create", Configuration, Values),'),
            "missing metadata row": (table, metadata.replace(last_row, "")),
            "incomplete metadata row": (table, metadata.replace(last_row, '    RightSet => ("right.set", RoleRight),\n')),
            "unsupported metadata row": (table, metadata.replace(last_row, '    RightSet => role_metadata(),\n')),
            "nonliteral metadata name": (table, metadata.replace(last_row, '    RightSet => (RIGHT_SET, RoleRight, Values),\n')),
            "duplicate metadata variant": (table, metadata.replace(last_row, '    RightAdd => ("right.set", RoleRight, Values),\n')),
            "duplicate metadata name": (table, metadata.replace(last_row, '    RightSet => ("right.add", RoleRight, Values),\n')),
        }
        for case, (source, role_metadata) in cases.items():
            with self.subTest(case=case), self.assertRaises(RuntimeError):
                self.registration_fixture(source, role_metadata)

    def test_unknown_or_incomplete_typed_metadata_cannot_silently_drop_support(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        support_metadata = (REPO_ROOT / "crates/unica-metadata/src/support.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n"
        table = roles + "(SupportOperation::CapabilitySet), (SupportOperation::RuleSet),"
        last_row = '    RuleSet => ("supportRule.set", SupportRule, Values),\n'
        self.assertIn(last_row, support_metadata)
        cases = {
            "unknown typed family": (table.replace("SupportOperation::CapabilitySet", "OtherOperation::CapabilitySet"), support_metadata),
            "unknown variant": (table.replace("SupportOperation::CapabilitySet", "SupportOperation::Missing"), support_metadata),
            "missing registration": (table.replace("(SupportOperation::RuleSet),", ""), support_metadata),
            "all typed registrations missing": (roles, support_metadata),
            "duplicate registration": (table + "\n(SupportOperation::CapabilitySet),", support_metadata),
            "missing metadata file": (table, None),
            "missing metadata table": (table, support_metadata.replace("\nsupport_operations!(", "\nprevious_support_operations!(")),
            "repeated metadata table": (table, support_metadata + '\nsupport_operations!(Extra => ("support.extra", Configuration, Values),);\n'),
            "empty metadata table": (table, "support_operations!();"),
            "unterminated metadata table": (table, 'support_operations!(CapabilitySet => ("supportCapability.set", Configuration, Values),'),
            "missing metadata row": (table, support_metadata.replace(last_row, "")),
            "incomplete metadata row": (table, support_metadata.replace(last_row, '    RuleSet => ("supportRule.set", SupportRule),\n')),
            "unsupported metadata row": (table, support_metadata.replace(last_row, '    RuleSet => support_metadata(),\n')),
            "nonliteral metadata name": (table, support_metadata.replace(last_row, '    RuleSet => (SUPPORT_RULE_SET, SupportRule, Values),\n')),
            "duplicate metadata variant": (table, support_metadata.replace(last_row, '    CapabilitySet => ("supportRule.set", SupportRule, Values),\n')),
            "duplicate metadata name": (table, support_metadata.replace(last_row, '    RuleSet => ("supportCapability.set", SupportRule, Values),\n')),
        }
        for case, (source, support_source) in cases.items():
            with self.subTest(case=case), self.assertRaises(RuntimeError):
                self.registration_fixture(
                    source, metadata, {"support_operation.rs": support_source}, complete_support=False,
                )

    def test_support_metadata_survives_split_impls_and_unrelated_helpers(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        support_metadata = (REPO_ROOT / "crates/unica-metadata/src/support.rs").read_text(encoding="utf-8")
        contract = "    pub fn contract(self) -> OperationContract {"
        self.assertEqual(support_metadata.count(contract), 1)
        support_metadata = support_metadata.replace(
            contract,
            '}\n\n// support_operations!(Commented => ("ignored.operation", Configuration, Values),);\n'
            'impl SupportOperation {\n'
            '    #[allow(dead_code)]\n'
            '    fn unrelated_metadata_hint(self) -> &\'static str { "support_operations!(NotATable)" }\n\n'
            + contract,
        )
        registry, _ = self.registration_fixture(
            "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),",
            metadata, {"support_operation.rs": support_metadata},
        )
        self.assertEqual([entry for entry in registry if entry[1] == "Support"], [
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ])

    def test_raw_typed_family_rows_cannot_bypass_complete_typed_registrations(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),"
        support = "(SupportOperation::CapabilitySet), (SupportOperation::RuleSet),"
        raw_roles = (
            '("role.create", Role, Configuration, Values),'
            '("right.add", Role, Role, Items),'
            '("right.set", Role, RoleRight, Values),'
        )
        raw_support = (
            '("supportCapability.set", Support, Configuration, Values),'
            '("supportRule.set", Support, SupportRule, Values),'
        )
        for table in [
            roles + support + raw_roles,
            roles + support + raw_support,
            raw_roles + support,
            roles + raw_support,
        ]:
            with self.subTest(table=table), self.assertRaises(RuntimeError):
                self.registration_fixture(table, metadata, complete_support=False)

    def test_metadata_table_locator_ignores_comments_strings_and_character_literals(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        decoy = 'role_operations!(Decoy => ("ignored.operation", Configuration, Values),);'
        prefix = (
            '// ' + decoy + '\n'
            '/* outer /* nested */ ' + decoy + ' */\n'
            'const ORDINARY: &str = ' + json.dumps(decoy) + ';\n'
            'const RAW: &str = r###"a quoted \" prefix ' + decoy + '"###;\n'
            'const RAW_BYTES: &[u8] = br##"a quoted \" prefix ' + decoy + '"##;\n'
            'fn unused_c_literal() { let _ = cr#"left " role_operations!(NotATable); right"#; }\n'
            'fn quote_marker() -> char { \'"\' }\n'
            'fn byte_quote_marker() -> u8 { b\'"\' }\n'
            "fn lifetime_probe<'a>(value: &'a str) -> &'a str { value }\n"
        )
        table = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),"
        expected, implemented = self.registration_fixture(table, metadata)
        for family in ["Role", "Support", "Metadata", "Form", "Mxl", "Event", "Dcs"]:
            family_prefix = prefix.replace("role_operations", f"{family.lower()}_operations")
            filename = f"{family.lower()}_operation.rs"
            family_metadata = self.DCS_FIXTURE_METADATA if family == "Dcs" else self.family_source(family)
            with self.subTest(family=family):
                role_source = family_prefix + metadata if family == "Role" else metadata
                actual, actual_implemented = self.registration_fixture(
                    table, role_source,
                    {} if family == "Role" else {filename: family_prefix + family_metadata},
                )
                self.assertEqual(actual, expected)
                self.assertEqual(actual_implemented, implemented)
                with self.assertRaises(RuntimeError):
                    self.registration_fixture(
                        table, family_prefix if family == "Role" else metadata,
                        {} if family == "Role" else {filename: family_prefix},
                    )


class AcceptanceCorpusScheduleTests(unittest.TestCase):
    """The parallel schedule runs every scenario once with its old isolation."""

    @staticmethod
    def corpus(*scenarios):
        return {"workspace": "a", "protocolVersion": "2025-03-26", "scenarios": [
            {"id": identifier, "workspace": workspace,
             "wire": [{"tool": "unica.apply", "args": {"executionToken": "$executionToken"}, "expect": ["ok"]}]
             if publishes else [{"tool": "unica.view", "args": {}, "expect": ["ok"]}] * steps}
            for identifier, workspace, publishes, steps in scenarios]}

    def test_a_segment_is_the_life_of_one_server(self) -> None:
        corpus = self.corpus(("A1", "a", False, 2), ("B1", "b", False, 1), ("A2", "a", True, 1),
                             ("A3", "a", False, 1), ("B2", "b", True, 1), ("B3", "b", False, 3))
        segments = [[scenario["id"] for _, scenario in segment] for segment in corpus_segments(corpus)]
        # Exactly the old server lives: a switch or a publication ends one.
        self.assertEqual(segments, [["A1"], ["B1"], ["A2"], ["A3"], ["B2"], ["B3"]])
        corpus = self.corpus(("A1", "a", False, 1), ("A2", "a", False, 1), ("A3", "a", True, 1), ("A4", "a", False, 1))
        self.assertEqual([[scenario["id"] for _, scenario in segment] for segment in corpus_segments(corpus)],
                         [["A1", "A2", "A3"], ["A4"]])

    def test_workers_share_segments_exactly_once_and_deterministically(self) -> None:
        corpus = self.corpus(*[(f"S{index}", "abc"[index % 3], index % 4 == 0, 1 + index % 5) for index in range(40)])
        segments = corpus_segments(corpus)
        for workers in (1, 2, 3, 4, 100):
            with self.subTest(workers=workers):
                batches = corpus_batches(corpus, workers)
                self.assertEqual(batches, corpus_batches(corpus, workers))
                self.assertEqual(len(batches), min(workers, len(segments)))
                dealt = [segment for batch in batches for segment in batch]
                self.assertCountEqual(dealt, segments)
                for batch in batches:
                    self.assertEqual(batch, sorted(batch, key=lambda segment: segment[0][0]))
        with self.assertRaises(ValueError):
            corpus_batches(corpus, 0)

    def test_worker_count_comes_from_the_environment_and_never_shares_a_plugin(self) -> None:
        with patch.dict(os.environ, {"UNICA_ACCEPTANCE_WORKERS": "3"}):
            self.assertEqual(corpus_workers(None), 3)
            self.assertEqual(corpus_workers(Path("/staged-plugin")), 1)
            self.assertEqual(corpus_workers(None, lambda *_: None), 1)
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("UNICA_ACCEPTANCE_WORKERS", None)
            self.assertEqual(corpus_workers(None), SOURCE_WORKERS)
        for raw in ("0", "-1", "two", ""):
            with self.subTest(raw=raw), patch.dict(os.environ, {"UNICA_ACCEPTANCE_WORKERS": raw}):
                with self.assertRaises(ValueError):
                    corpus_workers(None)

    def test_a_lost_segment_fails_the_run_instead_of_passing(self) -> None:
        started = []

        class Server:
            provider_failure_cache_is_default = False

            def __init__(self, workspace, *_):
                self.workspace = workspace
                started.append(workspace)

            def call(self, *_):
                return {}

            def close(self):
                pass

        corpus = self.corpus(("A1", "a", False, 1), ("A2", "a", True, 1), ("B1", "b", False, 1), ("A3", "a", False, 1))
        complete = corpus_batches

        def lossy(selected, workers):
            return [batch[1:] or batch for batch in complete(selected, workers)]

        runs = {"complete": complete, "lossy": lossy}
        for name, deal in runs.items():
            started.clear()
            with (self.subTest(deal=name),
                  patch.dict(os.environ, {"UNICA_ACCEPTANCE_WORKERS": "2"}),
                  patch(f"{__name__}.AcceptanceServer", Server),
                  patch(f"{__name__}.corpus_batches", deal),
                  patch(f"{__name__}.shutil.copytree", lambda _, path: path.mkdir(parents=True)),
                  patch(f"{__name__}.classify", return_value=("ok", "")),
                  patch(f"{__name__}.content_mismatches", return_value=[]),
                  patch(f"{__name__}.capture_values", return_value={})):
                if name == "complete":
                    run_corpus(self, corpus)
                    # Two workers, three segments: one start per segment.
                    self.assertEqual(len(started), 3)
                else:
                    with self.assertRaisesRegex(AssertionError, "did not cover the corpus exactly once"):
                        run_corpus(self, corpus)

    def test_a_failing_worker_stops_the_others_and_raises(self) -> None:
        calls = []
        lock = threading.Lock()
        failed = threading.Event()
        stopped = threading.Event()
        failing_worker = []

        class Server:
            provider_failure_cache_is_default = False

            def __init__(self, workspace, *_):
                self.workspace = workspace

            def call(self, *_):
                with lock:
                    calls.append(None)
                return {}

            def close(self):
                # `run_batch` sets `stop` before it closes the failing
                # worker's server: from here on the stop is observable.
                if failing_worker == [threading.get_ident()]:
                    stopped.set()

        def classify_or_fail(response, context):
            with lock:
                first = not failed.is_set()
                if first:
                    failed.set()
                    failing_worker.append(threading.get_ident())
            if first:
                raise RuntimeError("worker sentinel")
            # The other worker finishes its scenario only once the failure is
            # recorded, so its next scenario boundary must see the stop.
            self.assertTrue(stopped.wait(10), "the failing worker never closed its server")
            return "ok", ""

        # Every scenario switches the workspace, so each is its own segment and
        # each of the two workers holds twenty of them: without the stop the
        # other worker would make all twenty calls.
        corpus = self.corpus(*[(f"S{index}", "ab"[index % 2], False, 1) for index in range(40)])
        with (patch.dict(os.environ, {"UNICA_ACCEPTANCE_WORKERS": "2"}),
              patch(f"{__name__}.AcceptanceServer", Server),
              patch(f"{__name__}.shutil.copytree", lambda _, path: path.mkdir(parents=True)),
              patch(f"{__name__}.classify", classify_or_fail),
              patch(f"{__name__}.content_mismatches", return_value=[]),
              patch(f"{__name__}.capture_values", return_value={})):
            with self.assertRaisesRegex(RuntimeError, "worker sentinel"):
                run_corpus(self, corpus)
        # The failing call plus at most the one call the other worker had
        # already started; a run without the stop makes twenty-one.
        self.assertLessEqual(len(calls), 2, "the other worker must stop at its next scenario")


class AcceptanceCorpusRunTests(unittest.TestCase):
    maxDiff = None

    @classmethod
    def setUpClass(cls) -> None:
        ensure_binary(REPO_ROOT)
        cls.corpus = load_acceptance_corpus()
        for scenario in cls.corpus["scenarios"]:
            declared = set()
            for step in scenario["wire"]:
                validate_controls(step, declared)

    def test_s047_publishes_the_planned_comment_after_reading_the_object(self) -> None:
        scenario = next(s for s in self.corpus["scenarios"] if s["id"] == "S047")
        self.assertTrue(scenario_publishes(scenario), "the comment-writing task must execute its plan")
        planning = next(s["args"] for s in scenario["wire"] if "ops" in s["args"])
        expected = planning["ops"][0]["args"]["values"]["Comment"]
        with tempfile.TemporaryDirectory(prefix="unica-s047-") as raw:
            root = Path(raw).resolve()
            workspace = root / "workspace"
            shutil.copytree(REPO_ROOT / self.corpus["workspace"], workspace)
            state = root / "state"
            state.mkdir()

            def snapshot():
                return {p.relative_to(workspace / "src"): p.read_bytes()
                        for p in (workspace / "src").rglob("*") if p.is_file()}

            before = snapshot()
            context = {}
            server = AcceptanceServer(workspace, state, self.corpus["protocolVersion"])
            try:
                for step in scenario["wire"]:
                    arguments = substitute(step["args"], context)
                    response = server.call(step["tool"], arguments)
                    actual, note = classify(response, context)
                    self.assertTrue(matches(step["expect"], actual), note)
                    if "ops" in arguments:
                        self.assertEqual(snapshot(), before, "planning must not publish the comment")
                self.assertNotEqual(snapshot(), before, "the scenario must write its comment")
                response = server.call("unica.view", {"at": planning["at"]})
                result = response["result"]["structuredContent"]
                self.assertTrue(result["ok"], result)
                self.assertEqual(result["data"]["props"]["Comment"], expected)
            finally:
                server.close()
                server.process.stdout.close()

    def test_content_mismatch_stops_calls_and_does_not_commit_captures(self) -> None:
        self.corpus = {**self.corpus, "scenarios": [{
            "id": "content-counterexample", "area": "Properties", "task": "Counterexample",
            "wire": [
                {"tool": "unica.view", "args": {"at": "main:Enum.ВажностьПроблемыУчета"},
                 "expect": ["ok"], "captures": {"address": {"pointer": "/at"}},
                 "assertions": [{"pointer": "/data/props/Comment", "eq": "sentinel-never-published"}]},
                {"tool": "unica.view", "args": {"at": "$capture.address"}, "expect": ["ok"]},
            ],
        }]}
        original = AcceptanceServer.call
        called = []

        def observed(server, tool, arguments, label=None):
            called.append(tool)
            return original(server, tool, arguments, label)

        with patch.object(AcceptanceServer, "call", observed), patch(
            "tests.ci.test_acceptance_scenarios.capture_values", wraps=capture_values
        ) as capture:
            with self.assertRaisesRegex(AssertionError, "sentinel-never-published"):
                self.test_every_wire_answers_its_frozen_classes()
            self.assertEqual(called, ["unica.view"])
            capture.assert_not_called()

    def test_every_wire_answers_its_frozen_classes(self) -> None:
        run_corpus(self, select_profile(self.corpus, "source"))


class DeliveredAcceptanceServer(AcceptanceServer):
    def __init__(self, *args, **kwargs):
        self._closed = False
        try:
            super().__init__(*args, **kwargs)
        except BaseException:
            self.close()
            raise

    def close(self):
        if self._closed:
            return
        process = getattr(self, "process", None)
        try:
            if process is not None:
                try:
                    process.stdin.close()
                except OSError:
                    pass
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
                reader = getattr(self, "reader", None)
                if reader is not None:
                    reader.join(timeout=10)
                    if reader.is_alive():
                        raise TimeoutError("delivery stdout reader did not stop after MCP exit")
                process.stdout.close()
        finally:
            stderr = getattr(self, "stderr_file", None)
            if stderr is not None:
                stderr.close()
            self._closed = True


# Workers that run the `source` profile side by side.  Each owns its server,
# its workspace copies and whole workspace groups; `UNICA_ACCEPTANCE_WORKERS`
# overrides it for a local run, `1` runs in the calling thread.
SOURCE_WORKERS = 4
# A server start (copy, process, first call) costs about as much as this many
# steps; the weight balances the workers by expected time, not by count.
START_WEIGHT = 4


def corpus_segments(corpus) -> list[list[tuple[int, dict]]]:
    """Split the corpus into the lives of its servers.

    A server serves neighbouring scenarios of one workspace in corpus order;
    a scenario of another workspace or a publishing one ends it, and the
    next scenario starts on a fresh server over the pristine fixture.  A
    segment is that life exactly, so it is independent of every other one
    and keeps the state its scenarios always saw."""
    segments: list[list[tuple[int, dict]]] = []
    closed = True
    for ordinal, scenario in enumerate(corpus["scenarios"]):
        workspace = scenario.get("workspace", corpus["workspace"])
        if closed or workspace != segments[-1][0][1].get("workspace", corpus["workspace"]):
            segments.append([])
        segments[-1].append((ordinal, scenario))
        closed = scenario_publishes(scenario)
    return segments


def corpus_batches(corpus, workers: int) -> list[list[list[tuple[int, dict]]]]:
    """Deal segments to workers, heaviest first, onto the least loaded one.

    The deal depends only on the corpus and the worker count; every worker
    runs its segments in corpus order."""
    if workers < 1:
        raise ValueError("acceptance corpus needs at least one worker")

    def weight(segment):
        return START_WEIGHT + sum(len(scenario["wire"]) for _, scenario in segment)

    loads = [0] * workers
    dealt: list[list[list[tuple[int, dict]]]] = [[] for _ in range(workers)]
    for segment in sorted(corpus_segments(corpus), key=lambda segment: (-weight(segment), segment[0][0])):
        target = loads.index(min(loads))
        loads[target] += weight(segment)
        dealt[target].append(segment)
    return [sorted(batch, key=lambda segment: segment[0][0]) for batch in dealt if batch]


def corpus_workers(plugin, scenario_driver=None) -> int:
    if plugin is not None or scenario_driver is not None:
        # A delivered plugin is one staged installation, and a driver may
        # hold its own outside state: neither is shared between workers.
        return 1
    raw = os.environ.get("UNICA_ACCEPTANCE_WORKERS", str(SOURCE_WORKERS))
    if not raw.isdigit() or int(raw) < 1:
        raise ValueError(f"UNICA_ACCEPTANCE_WORKERS must be a positive integer: {raw!r}")
    return int(raw)


def run_corpus(test_case, corpus, plugin=None, scenario_driver=None, fixture_cleanup=None):
    from concurrent.futures import ThreadPoolExecutor
    from tests.ci.private_provider_failure_evidence import before_call, emit_failure_evidence

    default_workspace = corpus["workspace"]
    if not corpus["scenarios"]:
        raise ValueError("selected acceptance profile is empty")
    batches = corpus_batches(corpus, corpus_workers(plugin, scenario_driver))
    stop = threading.Event()
    segment_starts = {segment[0][0] for batch in batches for segment in batch}

    def fresh_server(root: Path, generation: int, workspace_relative: str) -> AcceptanceServer:
        home = root / f"run-{generation}"
        workspace = home / "workspace"
        shutil.copytree(REPO_ROOT / workspace_relative, workspace)
        if workspace_relative == "tests/fixtures/acceptance/workspace-search-wide":
            # Reverse directory traversal reaches ZNoise before the BSL.
            # Generate the capacity probe in the disposable workspace.
            noise = workspace / "src/ZNoise"
            noise.mkdir()
            for index in range(16_385):
                (noise / f"noise{index:05}.txt").touch()
        if workspace_relative == "tests/fixtures/acceptance/workspace-external-linked":
            materialize_external_form_link(workspace)
        if workspace_relative in {"tests/fixtures/acceptance/workspace-external", "tests/fixtures/acceptance/workspace-external-invalid", "tests/fixtures/acceptance/workspace-external-linked", "tests/fixtures/acceptance/workspace-external-missing"}:
            from tests.ci.acceptance_diagnostics import initialize_vendor_source
            initialize_vendor_source(workspace)
        if workspace_relative == FORMAT_WORKSPACE:
            derive_source_sets(REPO_ROOT / default_workspace / "src", workspace)
        state = home / "state"
        state.mkdir()
        environment = isolated_environment(plugin, state) if plugin else None
        server_type = DeliveredAcceptanceServer if plugin else AcceptanceServer
        return server_type(workspace, state, corpus["protocolVersion"], environment)

    def run_batch(root: Path, batch) -> tuple[list[int], list[tuple[int, str]]]:
        """Run one worker's segments; return the attempted ordinals and mismatches."""
        attempted: list[int] = []
        found_all: list[tuple[int, str]] = []
        generation = 0
        server = None
        try:
            for case_ordinal, scenario in (entry for segment in batch for entry in segment):
                if stop.is_set():
                    # Another worker failed; its exception decides the run.
                    break
                wanted = scenario.get("workspace", default_workspace)
                if server is not None and case_ordinal in segment_starts:
                    server.close()
                    server = None
                if server is None:
                    # A segment shares its server; a publishing or broken
                    # scenario ends it, and the next one starts from the
                    # pristine fixture so results never depend on order.
                    generation += 1
                    server = fresh_server(root, generation, wanted)
                attempted.append(case_ordinal)
                found: list[str] = []
                context = {"workspace": server.workspace}
                broken = False
                finish = scenario_driver(server, scenario) if scenario_driver else None
                verification = []
                for index, step in enumerate(scenario["wire"]):
                    label = f"{scenario['id']} step {index} {step['tool']}"
                    cache_is_default = getattr(server, "provider_failure_cache_is_default", False)
                    failure_baseline = before_call(server.workspace, cache_is_default=cache_is_default)
                    try:
                        arguments = substitute(step["args"], context)
                        if step.get("form"):
                            arguments = form_arguments(
                                server.input_schema(step["tool"]), arguments
                            )
                        response = (completed_delivery_call(server, step["tool"], arguments, label)
                                    if plugin or wanted == "tests/fixtures/acceptance/workspace-search-wide"
                                    else server.call(step["tool"], arguments, label))
                    except (TimeoutError, OSError, ValueError) as error:
                        # The session is gone: record the step with the
                        # daemon's own words and continue on a fresh one.
                        found.append(
                            f"{label}: session failed :: {error} :: "
                            f"stderr tail: {server.stderr_tail(400)}"
                        )
                        broken = True
                        break
                    candidate = dict(context)
                    actual, note = classify(response, candidate)
                    if not matches_step(step, actual):
                        found.append(
                            f"{label}: expected {step['expect']}, got {actual} :: {note[:160]}"
                        )
                        broken = True
                        break
                    # A refusal proves its exact answer, and a check over a
                    # node proves the verdict it froze; without these two
                    # checks a typo in an address or a failing validator
                    # would pass as the intended outcome.
                    structured = tool_result(response) or {}
                    verification.append({"tool": step["tool"], "args": arguments, "result": structured})
                    failures = content_mismatches(step, structured, actual, note, server.workspace, context)
                    if not failures:
                        try:
                            candidate["captures"] = capture_values(step, structured, server.workspace, context)
                        except (ValueError, OSError) as error:
                            failures.append(str(error))
                    if failures:
                        found.extend(f"{label}: {failure}" for failure in failures)
                        broken = True
                        break
                    # The independent evaluator needs the actual file values,
                    # not merely the fact that this harness compared them.
                    if step.get("fileAssertions"):
                        verification[-1]["fileObservations"] = [
                            {"descriptor": substitute(descriptor, context),
                             "observed": file_value(server.workspace, descriptor)}
                            for descriptor in step["fileAssertions"]
                        ]
                    context = candidate
                if broken:
                    emit_failure_evidence(
                        server.workspace, failure_baseline, cache_is_default=cache_is_default,
                        case_ordinal=case_ordinal, step_ordinal=index,
                    )
                if finish is not None and not broken:
                    finish(verification)
                found_all.extend((case_ordinal, mismatch) for mismatch in found)
                # The old run directory stays until the temporary directory
                # is removed, after the daemon's idle grace has passed.
                if broken or scenario_publishes(scenario):
                    server.close()
                    server = None
        except BaseException:
            stop.set()
            raise
        finally:
            if server is not None:
                server.close()
        return attempted, found_all

    mismatches: list[tuple[int, str]] = []
    attempted: list[int] = []
    with tempfile.TemporaryDirectory(
        prefix="unica-acceptance-", ignore_cleanup_errors=True
    ) as raw:
        root = Path(raw).resolve()
        roots = [root / f"worker-{index}" for index in range(len(batches))]
        try:
            if len(batches) == 1:
                outcomes = [run_batch(roots[0], batches[0])]
            else:
                with ThreadPoolExecutor(max_workers=len(batches)) as pool:
                    futures = [pool.submit(run_batch, worker_root, batch)
                               for worker_root, batch in zip(roots, batches, strict=True)]
                    outcomes = [future.result() for future in futures]
            for done, found in outcomes:
                attempted.extend(done)
                mismatches.extend(found)
        finally:
            # Servers are closed; owned fixtures go before their workspaces.
            if fixture_cleanup is not None:
                fixture_cleanup()
    # Every scenario of the selection runs exactly once, whatever the deal:
    # a lost worker or a dropped scenario fails here, not as a silent pass.
    test_case.assertEqual(sorted(attempted), list(range(len(corpus["scenarios"]))),
                          "the acceptance workers did not cover the corpus exactly once")
    test_case.assertEqual(
        [mismatch for _, mismatch in sorted(mismatches, key=lambda item: item[0])],
        [],
        "the surface answered outside the frozen acceptance classes:\n"
        + "\n".join(mismatch for _, mismatch in sorted(mismatches, key=lambda item: item[0])),
    )


class AcceptanceDeliveryCorpusRunTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        ensure_binary(REPO_ROOT)
        cls.corpus = load_acceptance_corpus()
        for scenario in cls.corpus["scenarios"]:
            declared = set()
            for step in scenario["wire"]:
                validate_controls(step, declared)

    def test_analyzer_delivery_wires_have_terminal_content(self):
        import sys
        corpus = select_profile(self.corpus, "delivery")
        with tempfile.TemporaryDirectory(prefix="unica-analyzer-delivery-") as raw:
            plugin = Path(raw).resolve() / "plugin"
            subprocess.run([sys.executable, "-m", "tests.ci.acceptance_profiles", "--stage-analyzer", str(plugin)],
                           cwd=REPO_ROOT, timeout=90, check=True)
            from tests.ci.acceptance_diagnostics import prepare_diagnostic_case
            with ExitStack() as fixtures:
                def prepare(server, scenario):
                    return prepare_diagnostic_case(server, scenario, plugin=plugin, fixtures=fixtures)
                run_corpus(self, corpus, plugin, scenario_driver=prepare,
                           fixture_cleanup=fixtures.close)
    def test_pinned_resident_reports_real_baseline_author_and_scope_facts(self):
        from tests.ci.acceptance_profiles import stage_analyzer, host_target
        from tests.ci.acceptance_diagnostics import resident_filter_evidence
        import platform
        with tempfile.TemporaryDirectory(prefix="unica-resident-filter-") as raw:
            root=Path(raw).resolve()
            plugin=stage_analyzer(root / "plugin")
            lock=json.loads((plugin / "third-party/tools.lock.json").read_text())
            target=host_target(lock,platform.system(),platform.machine())
            binary=plugin / "bin" / target / ("bsl-analyzer.exe" if os.name=="nt" else "bsl-analyzer")
            workspace=root / "workspace"
            shutil.copytree(REPO_ROOT / DIAGNOSTICS_WORKSPACE,workspace)
            evidence=resident_filter_evidence(binary,workspace,root)
            self.assertEqual(len(evidence["plain"]["result"]["findings"]),3)
            self.assertEqual(evidence["authors"]["result"]["findings_ignored_by_author"],3)
            self.assertEqual(evidence["authors"]["result"]["findings"],[])
            self.assertIs(evidence["scope"]["result"]["out_of_scope"],True)
            self.assertEqual(evidence["both"]["result"]["baseline"]["known"],1)
            self.assertEqual(evidence["both"]["result"]["baseline"]["new"],2)
            self.assertEqual(evidence["both"]["result"]["findings_ignored_by_author"],2)
            self.assertEqual(evidence["both"]["result"]["findings"],[])



class AcceptanceFaultCorpusRunTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        ensure_binary(REPO_ROOT)
        cls.corpus = load_acceptance_corpus()

    def test_controlled_jsonl_faults_preserve_public_reason_without_provider_input(self):
        from tests.ci.acceptance_faults import stage_fault_producer, fault_driver
        with tempfile.TemporaryDirectory(prefix="unica-jsonl-fault-") as raw:
            plugin = Path(raw).resolve() / "plugin"
            manifest = stage_fault_producer(plugin)
            self.assertFalse(manifest["testFixture"]["publishedArtifact"])
            self.assertEqual(manifest["tools"][0]["version"], "0.0.0-test-fixture")
            records = []
            run_corpus(self, fault_cases(self.corpus, "analyzer-jsonl-fault"), plugin,
                       scenario_driver=fault_driver(plugin, records))
            self.assertEqual(sum(record["tool"] == "unica.check" for record in records), 6)

    def test_call_graph_branch_names_a_failed_analyzer_delivery(self):
        from tests.ci.acceptance_faults import stage_missing_analyzer, delivery_failure_driver
        with tempfile.TemporaryDirectory(prefix="unica-delivery-failure-") as raw:
            plugin = Path(raw).resolve() / "plugin"
            manifest = stage_missing_analyzer(plugin)
            self.assertFalse(manifest["testFixture"]["publishedArtifact"])
            cases = fault_cases(self.corpus, "analyzer-delivery-failure")
            self.assertEqual([scenario["id"] for scenario in cases["scenarios"]], ["S396"])
            run_corpus(self, cases, plugin, scenario_driver=delivery_failure_driver(plugin))


def fault_cases(corpus, driver):
    selected = select_profile(corpus, "fault-injection")
    return {**selected, "scenarios": [
        scenario for scenario in selected["scenarios"] if scenario["driver"] == driver
    ]}


if __name__ == "__main__":
    unittest.main()
