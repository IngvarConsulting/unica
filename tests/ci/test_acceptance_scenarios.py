"""Acceptance corpus: real developer tasks ride the canonical surface.

The common corpus has source, pinned-analyzer delivery and real-agent drivers.
Runtime drivers are implemented with their surface tasks;
a profile without an executable driver is rejected.

Every scenario in tests/fixtures/acceptance/scenario-corpus.json is a real
configuration-development task expressed as a wire of canonical unica.* calls
against the acceptance fixture workspace.  The corpus freezes, per step, the
class of answer the surface gives today:

  ok          ok:true result without a task receipt
  task        ok:true queued/working Task receipt
  unsupported typed refusal whose code starts with unsupported_
  provider    typed provider_unavailable refusal
  cancelled   typed task_cancelled outcome of an intended cancellation
  failed      typed task_failed terminal outcome
  refused     an intended bad_value probe of refusal quality
  gap         a documented non-passable spot with its reason

A wire is passable when every step answers its frozen class; a raw error, an
unknown tool, or an undocumented bad_value fails the run.  A step marked
`form: true` is sent the way a form client sends it: every published schema
default fills an argument the step leaves out.  Environment-shaped
steps (docs search, platform runs, task follow-ups) freeze the set of classes
the environment legitimately selects from.
"""
from __future__ import annotations

import importlib.util
import json
import queue
import re
import threading
import time
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
    file_value,
    load_corpus,
    matches_step,
    substitute_capture,
    validate_controls,
)

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS = REPO_ROOT / "tests/fixtures/acceptance/scenario-corpus.json"
BINARY = (
    (REPO_ROOT / Path(os.environ.get("CARGO_TARGET_DIR", "target"))).resolve()
    / "debug"
    / ("unica.exe" if os.name == "nt" else "unica")
)
from tests.ci.acceptance_profiles import (
    DIAGNOSTICS_WORKSPACE, SYMBOL_WORKSPACE, completed_delivery_call, isolated_environment, select_profile,
)

REFUSAL_CODES = {"bad_value", "not_found", "invalid_state", "invalid_source", "stale_revision"}

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
    structured = response.get("result", {}).get("structuredContent")
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
    if code.startswith("unsupported_"):
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


def matches(expected: list[str], actual: str) -> bool:
    if actual in expected:
        return True
    # A documented gap freezes today's non-passable answer; a refusal probe
    # freezes an intended bad_value.  Both arrive through the same classes.
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
        corpus = load_corpus(CORPUS)
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
        cls.corpus = load_corpus(CORPUS)

    def test_corpus_is_uniquely_numbered_and_not_missing_steps(self) -> None:
        scenarios = self.corpus["scenarios"]
        self.assertEqual(len(scenarios), 357)
        # Исполнение apply следует за планированием с сохранением токена.
        self.assertEqual(sum(len(scenario["wire"]) for scenario in scenarios), 598,
            "a wire step went missing: the corpus freezes 598 steps",
        )
        identifiers = [scenario["id"] for scenario in scenarios]
        self.assertEqual(identifiers, [f"S{index:03d}" for index in range(1, 358)])

    def test_all_scenarios_have_an_executable_profile(self) -> None:
        source = {s["id"] for s in select_profile(self.corpus, "source")["scenarios"]}
        delivery = {s["id"] for s in select_profile(self.corpus, "delivery")["scenarios"]}
        agent = {s["id"] for s in select_profile(self.corpus, "agent-evaluation")["scenarios"]}
        fault = {s["id"] for s in select_profile(self.corpus, "fault-injection")["scenarios"]}
        self.assertEqual(fault, {"S344", "S345", "S346"})
        self.assertFalse(fault & (source | delivery | agent))
        self.assertFalse(source & delivery or source & agent or delivery & agent)
        self.assertEqual(source | delivery | agent | fault, {s["id"] for s in self.corpus["scenarios"]})
        self.assertEqual(agent, {"S328", "S335"})
        self.assertEqual(delivery, {"S324", "S339", "S340", "S341", "S342", "S343"})

    def test_every_step_freezes_known_classes_and_documents_gaps(self) -> None:
        for scenario in self.corpus["scenarios"]:
            declared = set()
            workspace = scenario.get("workspace", self.corpus["workspace"])
            self.assertIn(
                workspace,
                {self.corpus["workspace"], FORMAT_WORKSPACE, BARE_WORKSPACE, SYMBOL_WORKSPACE, DIAGNOSTICS_WORKSPACE, "tests/fixtures/acceptance/workspace-dcs", "tests/fixtures/acceptance/workspace-mxl", "tests/fixtures/acceptance/workspace-code", "tests/fixtures/acceptance/workspace-metadata-warning", "tests/fixtures/acceptance/workspace-metadata-values", "tests/fixtures/acceptance/workspace-external", "tests/fixtures/acceptance/workspace-external-invalid", "tests/fixtures/acceptance/workspace-external-linked"},
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

    def test_gaps_stay_a_bounded_exception_not_a_habit(self) -> None:
        gap_steps = [
            (scenario["id"], step)
            for scenario in self.corpus["scenarios"]
            for step in scenario["wire"]
            if "gap" in step["expect"]
        ]
        # Every documented gap names a reproduced surface defect in its text.
        # The rendered registry lists them; the ceiling from the fixture README
        # (eight) is a ratchet against silent growth, not a target.
        self.assertLessEqual(
            len(gap_steps),
            8,
            "documented gaps grew: either fix the surface or re-approve the corpus",
        )


class AcceptanceRegistryDocumentTests(unittest.TestCase):
    """`docs/acceptance-scenarios.md` is the rendered view of the corpus for
    contributors; it must never drift from the JSON it is generated from."""

    def test_registry_document_is_rendered_from_the_corpus(self) -> None:
        module_path = REPO_ROOT / "scripts" / "ci" / "render-acceptance-registry.py"
        spec = importlib.util.spec_from_file_location("render_acceptance_registry", module_path)
        self.assertIsNotNone(spec)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        rendered = module.render_document()
        document = REPO_ROOT / "docs" / "acceptance-scenarios.md"
        self.assertEqual(
            document.read_text(encoding="utf-8"),
            rendered,
            "docs/acceptance-scenarios.md is stale; run "
            "`python scripts/ci/render-acceptance-registry.py --write`",
        )
        self.assertIn("## Покрытие реестра `apply`", rendered)


class AcceptanceCorpusRunTests(unittest.TestCase):
    maxDiff = None

    @classmethod
    def setUpClass(cls) -> None:
        subprocess.run(
            ["cargo", "build", "--quiet", "--package", "unica-coder", "--bin", "unica"],
            cwd=REPO_ROOT,
            check=True,
        )
        cls.corpus = load_corpus(CORPUS)
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


def run_corpus(test_case, corpus, plugin=None, scenario_driver=None):
    default_workspace = corpus["workspace"]
    mismatches = []
    with tempfile.TemporaryDirectory(
        prefix="unica-acceptance-", ignore_cleanup_errors=True
    ) as raw:
        root = Path(raw).resolve()
        generation = 0

        def fresh_server(workspace_relative: str) -> AcceptanceServer:
            nonlocal generation
            generation += 1
            home = root / f"run-{generation}"
            workspace = home / "workspace"
            shutil.copytree(REPO_ROOT / workspace_relative, workspace)
            if workspace_relative == "tests/fixtures/acceptance/workspace-external-linked":
                materialize_external_form_link(workspace)
            if workspace_relative in {"tests/fixtures/acceptance/workspace-external", "tests/fixtures/acceptance/workspace-external-invalid", "tests/fixtures/acceptance/workspace-external-linked"}:
                from tests.ci.acceptance_diagnostics import initialize_vendor_source
                initialize_vendor_source(workspace)
            if workspace_relative == FORMAT_WORKSPACE:
                derive_source_sets(REPO_ROOT / default_workspace / "src", workspace)
            state = home / "state"
            state.mkdir()
            environment = isolated_environment(plugin, state) if plugin else None
            server_type = DeliveredAcceptanceServer if plugin else AcceptanceServer
            return server_type(workspace, state, corpus["protocolVersion"], environment)

        if not corpus["scenarios"]:
            raise ValueError("selected acceptance profile is empty")
        current_workspace = corpus["scenarios"][0].get("workspace", default_workspace)
        server = fresh_server(current_workspace)
        try:
            for scenario in corpus["scenarios"]:
                wanted = scenario.get("workspace", default_workspace)
                if wanted != current_workspace:
                    # A scenario may address another fixture workspace;
                    # the session is bound to one, so it starts over.
                    server.close()
                    current_workspace = wanted
                    server = fresh_server(current_workspace)
                context = {}
                broken = False
                finish = scenario_driver(server, scenario) if scenario_driver else None
                verification = []
                for index, step in enumerate(scenario["wire"]):
                    label = f"{scenario['id']} step {index} {step['tool']}"
                    try:
                        arguments = substitute(step["args"], context)
                        if step.get("form"):
                            arguments = form_arguments(
                                server.input_schema(step["tool"]), arguments
                            )
                        response = (completed_delivery_call(server, step["tool"], arguments, label)
                                    if plugin else server.call(step["tool"], arguments, label))
                    except (TimeoutError, OSError, ValueError) as error:
                        # The session is gone: record the step with the
                        # daemon's own words and continue on a fresh one.
                        mismatches.append(
                            f"{label}: session failed :: {error} :: "
                            f"stderr tail: {server.stderr_tail(400)}"
                        )
                        broken = True
                        break
                    candidate = dict(context)
                    actual, note = classify(response, candidate)
                    if not matches_step(step, actual):
                        mismatches.append(
                            f"{label}: expected {step['expect']}, got {actual} :: {note[:160]}"
                        )
                        broken = True
                        break
                    # A refusal proves its exact answer, and a check over a
                    # node proves the verdict it froze; without these two
                    # checks a typo in an address or a failing validator
                    # would pass as the intended outcome.
                    structured = response.get("result", {}).get("structuredContent") or {}
                    verification.append({"tool": step["tool"], "args": arguments, "result": structured})
                    failures = content_mismatches(step, structured, actual, note, server.workspace, context)
                    if not failures:
                        try:
                            candidate["captures"] = capture_values(step, structured, server.workspace, context)
                        except (ValueError, OSError) as error:
                            failures.append(str(error))
                    if failures:
                        mismatches.extend(f"{label}: {failure}" for failure in failures)
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
                if finish is not None and not broken:
                    finish(verification)
                # A scenario that published changes leaves its mark on the
                # workspace; the next one starts from the pristine fixture
                # so results never depend on corpus order. The old run
                # directory stays until the temporary directory is removed,
                # after the daemon's idle grace has passed.
                if broken or scenario_publishes(scenario):
                    server.close()
                    server = fresh_server(current_workspace)
        finally:
            server.close()
    test_case.assertEqual(
        mismatches,
        [],
        "the surface answered outside the frozen acceptance classes:\n"
        + "\n".join(mismatches),
    )


class AcceptanceDeliveryCorpusRunTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(["cargo", "build", "--quiet", "--locked", "--package", "unica-coder", "--bin", "unica"],
                       cwd=REPO_ROOT, check=True)
        cls.corpus = load_corpus(CORPUS)
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
            run_corpus(self, corpus, plugin, scenario_driver=prepare_diagnostic_case)
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
        subprocess.run(["cargo", "build", "--quiet", "--locked", "--package", "unica-coder", "--bin", "unica"],
                       cwd=REPO_ROOT, check=True)
        cls.corpus = load_corpus(CORPUS)

    def test_controlled_jsonl_faults_preserve_public_reason_without_provider_input(self):
        from tests.ci.acceptance_faults import stage_fault_producer, fault_driver
        with tempfile.TemporaryDirectory(prefix="unica-jsonl-fault-") as raw:
            plugin = Path(raw).resolve() / "plugin"
            manifest = stage_fault_producer(plugin)
            self.assertFalse(manifest["testFixture"]["publishedArtifact"])
            self.assertEqual(manifest["tools"][0]["version"], "0.0.0-test-fixture")
            records = []
            run_corpus(self, select_profile(self.corpus, "fault-injection"), plugin,
                       scenario_driver=fault_driver(plugin, records))
            self.assertEqual(sum(record["tool"] == "unica.check" for record in records), 3)


if __name__ == "__main__":
    unittest.main()
