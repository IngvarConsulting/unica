"""Acceptance corpus: real developer tasks ride the canonical surface.

The run dictionary and Task lifecycle are excluded on purpose: that half of
the surface is being built separately and gets its own acceptance corpus.

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
unknown tool, or an undocumented bad_value fails the run.  Environment-shaped
steps (docs search, platform runs, task follow-ups) freeze the set of classes
the environment legitimately selects from.
"""
from __future__ import annotations

import importlib.util
import json
import queue
import re
import threading
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS = REPO_ROOT / "tests/fixtures/acceptance/scenario-corpus.json"
BINARY = (
    Path(os.environ.get("CARGO_TARGET_DIR", REPO_ROOT / "target"))
    / "debug"
    / ("unica.exe" if os.name == "nt" else "unica")
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
    if isinstance(value, str):
        if value == "$task":
            return context.get("task", "00000000-0000-4000-8000-000000000000")
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
    task = (structured.get("data") or {}).get("task") or {}
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


def derive_source_sets(source: Path, workspace: Path) -> None:
    """Derive the format-probe source sets of `workspace-format/` from `src`.

    `v8project.yaml` of that fixture declares three sets and no `main`:
    `newer` is the same tree with every 2.20 root rewritten to 2.21 (and the
    Reports and XDTO packages dropped, because the strict read port cannot
    open a 2.21 template wrapper), `nosupport` is `src` without
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

    def unversioned(path: Path, text: str) -> str:
        if path.name == "Configuration.xml":
            text = re.sub(r'(<MetaDataObject[^>]*?) version="2\.20"', r"\1", text, count=1)
        return text

    copy("src-newer", newer, drop_dirs=("Reports", "XDTOPackages"))
    copy("src-nosupport", lambda path, text: text, drop_bin=True)
    copy("src-unversioned", unversioned)

def scenario_publishes(scenario) -> bool:
    """True when a step can change the workspace: an apply that is not a preview."""
    return any(
        step["tool"] == "unica.apply" and not step["args"].get("dryRun", False)
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

    def __init__(self, cwd: Path, state: Path, protocol: str):
        env = dict(os.environ)
        env["UNICA_PROVIDER_STATE_DIR"] = str(state)
        self.stderr_path = state / "unica-stderr.log"
        self.stderr_file = open(self.stderr_path, "wb")  # noqa: SIM115 - lives as long as the process
        self.process = subprocess.Popen(
            [str(BINARY)],
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

    def receive(self, expected_id: int | None = None):
        while True:
            try:
                line = self.lines.get(timeout=RESPONSE_TIMEOUT_SECONDS)
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

    def call(self, tool: str, arguments, label: str | None = None):
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
        return self.receive(request_id)

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
        try:
            self.stderr_file.close()
        except OSError:
            pass


class AcceptanceCorpusShapeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.corpus = json.loads(CORPUS.read_text(encoding="utf-8"))

    def test_corpus_holds_the_run_free_scenario_set_uniquely_numbered(self) -> None:
        scenarios = self.corpus["scenarios"]
        self.assertEqual(len(scenarios), 288)
        self.assertEqual(sum(len(scenario["wire"]) for scenario in scenarios), 319,
            "a wire step went missing: the corpus freezes 319 steps",
        )
        identifiers = [scenario["id"] for scenario in scenarios]
        self.assertEqual(identifiers, [f"S{index:03d}" for index in range(1, 289)])

    def test_the_run_half_of_the_surface_stays_out_of_this_corpus(self) -> None:
        for scenario in self.corpus["scenarios"]:
            for step in scenario["wire"]:
                self.assertNotIn(
                    step["tool"],
                    {"unica.run", "unica.task.get", "unica.task.result", "unica.task.cancel"},
                    f"{scenario['id']}: run and Task acceptance lives in its own corpus",
                )

    def test_every_step_freezes_known_classes_and_documents_gaps(self) -> None:
        for scenario in self.corpus["scenarios"]:
            workspace = scenario.get("workspace", self.corpus["workspace"])
            self.assertIn(
                workspace,
                {self.corpus["workspace"], FORMAT_WORKSPACE},
                f"{scenario['id']}: a scenario runs on one of the two fixture workspaces",
            )
            for index, step in enumerate(scenario["wire"]):
                with self.subTest(scenario=scenario["id"], step=index):
                    self.assertTrue(step["tool"].startswith("unica."))
                    expected = step["expect"]
                    self.assertTrue(expected, "every step freezes an expectation")
                    self.assertTrue(set(expected) <= EXPECT_CLASSES, expected)
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
        # The corpus carries none today; the ceiling from the fixture README
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
        cls.corpus = json.loads(CORPUS.read_text(encoding="utf-8"))

    def test_every_wire_answers_its_frozen_classes(self) -> None:
        corpus = self.corpus
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
                if workspace_relative == FORMAT_WORKSPACE:
                    derive_source_sets(REPO_ROOT / default_workspace / "src", workspace)
                state = home / "state"
                state.mkdir()
                return AcceptanceServer(workspace, state, corpus["protocolVersion"])

            current_workspace = default_workspace
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
                    for index, step in enumerate(scenario["wire"]):
                        label = f"{scenario['id']} step {index} {step['tool']}"
                        arguments = substitute(step["args"], context)
                        try:
                            response = server.call(step["tool"], arguments, label)
                        except (TimeoutError, OSError, ValueError) as error:
                            # The session is gone: record the step with the
                            # daemon's own words and continue on a fresh one.
                            mismatches.append(
                                f"{label}: session failed :: {error} :: "
                                f"stderr tail: {server.stderr_tail(400)}"
                            )
                            broken = True
                            break
                        actual, note = classify(response, context)
                        if not matches(step["expect"], actual):
                            mismatches.append(
                                f"{label}: expected {step['expect']}, got {actual} :: {note[:160]}"
                            )
                            continue
                        # A refusal proves its exact answer, and a check over a
                        # node proves the verdict it froze; without these two
                        # checks a typo in an address or a failing validator
                        # would pass as the intended outcome.
                        if actual == "refused" and note != step.get("refusal"):
                            mismatches.append(
                                f"{label}: expected refusal {step.get('refusal')!r}, got {note!r}"
                            )
                        if "status" in step and actual == "ok":
                            structured = response.get("result", {}).get("structuredContent") or {}
                            verdict = (structured.get("data") or {}).get("status")
                            if verdict != step["status"]:
                                mismatches.append(
                                    f"{label}: expected validation status {step['status']!r}, "
                                    f"got {verdict!r}"
                                )
                        if "validators" in step and actual == "ok":
                            structured = response.get("result", {}).get("structuredContent") or {}
                            validators = (structured.get("data") or {}).get("validators")
                            if validators != step["validators"]:
                                mismatches.append(
                                    f"{label}: expected validators {step['validators']!r}, "
                                    f"got {validators!r}"
                                )
                        if "diagnostic" in step and actual == "ok":
                            # A validation step may freeze one diagnostic code it
                            # expects next to the verdict: the export-format guard
                            # of a root outside the active profile.
                            structured = response.get("result", {}).get("structuredContent") or {}
                            codes = [
                                diagnostic.get("code")
                                for diagnostic in (
                                    (structured.get("data") or {}).get("diagnostics") or []
                                )
                            ]
                            if step["diagnostic"] not in codes:
                                mismatches.append(
                                    f"{label}: expected diagnostic {step['diagnostic']!r}, "
                                    f"got {codes!r}"
                                )
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
        self.assertEqual(
            mismatches,
            [],
            "the surface answered outside the frozen acceptance classes:\n"
            + "\n".join(mismatches),
        )


if __name__ == "__main__":
    unittest.main()
