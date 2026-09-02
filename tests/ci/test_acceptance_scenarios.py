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

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS = REPO_ROOT / "tests/fixtures/acceptance/scenario-corpus.json"
BINARY = REPO_ROOT / "target/debug/unica"

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
    if response is None or "error" in response:
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
    if code == "bad_value":
        return "refused", f"{code}: {message[:160]}"
    return "gap-candidate", f"{code}: {message[:200]}"


def matches(expected: list[str], actual: str) -> bool:
    if actual in expected:
        return True
    # A documented gap freezes today's non-passable answer; a refusal probe
    # freezes an intended bad_value.  Both arrive through the same classes.
    if "gap" in expected and actual in {"refused", "gap-candidate", "provider"}:
        return True
    return False


class AcceptanceServer:
    def __init__(self, cwd: Path, state: Path, protocol: str):
        env = dict(os.environ)
        env["UNICA_PROVIDER_STATE_DIR"] = str(state)
        self.process = subprocess.Popen(
            [str(BINARY)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            cwd=cwd,
            env=env,
        )
        self.next_id = 0
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

    def receive(self):
        while True:
            line = self.process.stdout.readline()
            if not line:
                return None
            line = line.strip()
            if line:
                payload = json.loads(line)
                if "id" in payload:
                    return payload

    def call(self, tool: str, arguments):
        self.send(
            {
                "jsonrpc": "2.0",
                "id": self.request_id(),
                "method": "tools/call",
                "params": {"name": tool, "arguments": arguments},
            }
        )
        return self.receive()

    def close(self) -> None:
        try:
            self.process.stdin.close()
        except OSError:
            pass
        self.process.terminate()


class AcceptanceCorpusShapeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.corpus = json.loads(CORPUS.read_text(encoding="utf-8"))

    def test_corpus_holds_the_run_free_scenario_set_uniquely_numbered(self) -> None:
        scenarios = self.corpus["scenarios"]
        self.assertEqual(len(scenarios), 265)
        identifiers = [scenario["id"] for scenario in scenarios]
        self.assertEqual(identifiers, [f"S{index:03d}" for index in range(1, 266)])

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
                        self.assertTrue(
                            step.get("refusal"),
                            "an intended refusal freezes its message",
                        )

    def test_gaps_stay_a_bounded_exception_not_a_habit(self) -> None:
        gap_steps = [
            (scenario["id"], step)
            for scenario in self.corpus["scenarios"]
            for step in scenario["wire"]
            if "gap" in step["expect"]
        ]
        # Each gap names a distinct, reproduced surface defect (see the gap
        # text): typed contracts that cannot express indexing, the missing
        # role/subsystem property writers, the DocumentJournal column emitter,
        # the template reader/writer mismatch, and the DefinedType post-image
        # projection. The ceiling is a ratchet against silent growth, not a
        # target; lower it as each defect is fixed.
        self.assertLessEqual(
            len(gap_steps),
            9,
            "documented gaps grew: either fix the surface or re-approve the corpus",
        )


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
        workspace_source = REPO_ROOT / corpus["workspace"]
        mismatches = []
        with tempfile.TemporaryDirectory(prefix="unica-acceptance-") as raw:
            root = Path(raw).resolve()
            workspace = root / "workspace"
            shutil.copytree(workspace_source, workspace)
            state = root / "state"
            state.mkdir()
            server = AcceptanceServer(workspace, state, corpus["protocolVersion"])
            try:
                for scenario in corpus["scenarios"]:
                    context = {}
                    for index, step in enumerate(scenario["wire"]):
                        arguments = substitute(step["args"], context)
                        response = server.call(step["tool"], arguments)
                        actual, note = classify(response, context)
                        if not matches(step["expect"], actual):
                            mismatches.append(
                                f"{scenario['id']} step {index} {step['tool']}: "
                                f"expected {step['expect']}, got {actual} :: {note[:160]}"
                            )
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
