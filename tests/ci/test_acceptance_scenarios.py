"""Acceptance corpus: real developer tasks ride the canonical surface.

The common corpus has source, pinned-analyzer delivery and real-agent drivers.
Runtime drivers are implemented with their surface tasks;
a profile without an executable driver is rejected.

Every scenario of the corpus (one file per scenario in
tests/fixtures/acceptance/scenarios/, the closed block S001-S440 among them,
loaded with the header tests/fixtures/acceptance/scenario-corpus.json by
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
  invalid-result typed invalid_result terminal, used by controlled runtime faults
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
from scripts.ci.temp_tree import remove_tree, temp_tree

import importlib.util
import itertools
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

# Размер тестов модуля (scripts/ci/python_sizes.py).
PR_SKIP = "малым тестам нужен продукт"
# Приёмочный корпус: собирает продукт и гоняет сценарии через MCP.
MEDIUM = ("AcceptanceCorpusRunTests",)

REPO_ROOT = Path(__file__).resolve().parents[2]
# What each `apply` family declares about its operations, one file per family,
# written by hand: the registry read from Rust must agree with it by key.
APPLY_DECLARATIONS = REPO_ROOT / "tests/ci/fixtures/apply-registry"


def declared_operations(family):
    """The operations of an `apply` family as its declaration states them, in family order."""
    document = json.loads((APPLY_DECLARATIONS / f"{family.lower()}.json").read_text(encoding="utf-8"))
    if document.get("family") != family:
        raise AssertionError(f"{family.lower()}.json declares {document.get('family')!r}, not {family!r}")
    return document["operations"]


def declared_rows(*families):
    """Registry rows `(operation, family, applicability, skeleton)` the declarations state."""
    return [
        (operation["operation"], family, operation["applicability"], operation["skeleton"])
        for family in families
        for operation in declared_operations(family)
    ]


def declared_variants(family):
    """`(variant, operation, applicability, skeleton)` of a family, as its typed metadata names them."""
    return [
        (operation["variant"], operation["operation"], operation["applicability"], operation["skeleton"])
        for operation in declared_operations(family)
    ]


def declared_registrations(operation_type, family):
    """A composition-table fragment registering every declared variant of a family once."""
    return "".join(f"({operation_type}::{operation['variant']})," for operation in declared_operations(family))
BINARY = binary_path(REPO_ROOT)

from tests.ci.form_runtime_diagnostics import WORKSPACE as FORM_RUNTIME_WORKSPACE

from tests.ci.acceptance_profiles import (
    DIAGNOSTICS_WORKSPACE, SOURCE_WORKSPACES, SOURCE_WORKSPACE_PROFILES, SYMBOL_WORKSPACE, completed_delivery_call, isolated_environment,
    prepare_source_workspace, select_profile, HOST_CONTEXT_MISSING_WORKSPACE, missing_host_context_environment,
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
    "invalid-result",
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
    if code == "invalid_result":
        return "invalid-result", message[:160]
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
    `newer` is the same tree with every 2.20 root rewritten to 2.23 (and the
    Reports and XDTO packages dropped, because the strict read port cannot
    open a 2.23 template wrapper), `older` is the same tree at 2.17 — the
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
        text = text.replace('version="2.20"', 'version="2.23"')
        if path.name == "Configuration.xml":
            text = re.sub(r"<(Report|XDTOPackage)>[^<]+</\1>", "", text)
        return text

    def older(path: Path, text: str) -> str:
        text = text.replace('version="2.20"', 'version="2.17"')
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


def names_form_attribute(address: str) -> bool:
    segments = re.split(r"(?<!\\)\.", address.partition(":")[2])
    return (len(segments) == 6 and segments[2] == "Form"
            and segments[4] == "Attribute" and all(segments[index] for index in (1, 3, 5)))


def names_form_column(address: str) -> bool:
    segments = re.split(r"(?<!\\)\.", address.partition(":")[2])
    return (len(segments) == 8 and segments[2] == "Form"
            and segments[4] == "Attribute" and segments[6] == "Column"
            and all(segments[index] for index in (1, 3, 5, 7)))


def pins_published_file(assertion) -> bool:
    """True when a file assertion fixes what the publication wrote: exact
    bytes, XML content or a removed path.  `notEq`, a still-present path or
    the digest of an earlier capture tell nothing about the new state."""
    if assertion["kind"] == "exists":
        return assertion.get("eq") is False
    if assertion["kind"] == "sha256":
        return isinstance(assertion.get("eq"), str) and not assertion["eq"].startswith("$")
    return assertion["kind"] in {"xml", "xmlTags"} and (
        "eq" in assertion or "contains" in assertion or "sameItems" in assertion
    )


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
                    # Args captured from an earlier answer (`$capture.*`) are
                    # not known statically; the step address names the target.
                    args = operation.get("args")
                    target = (args.get("at") if isinstance(args, dict) else None) or step["args"]["at"]
                    on_form = names_form(target)
                    candidates = [row for row in candidates if (
                        row[1] == "Form" and on_form is True
                        or row[1] == "FormProperties" and (on_form is True or names_form_column(target))
                        or row[1] == "FormAttribute" and names_form_attribute(target)
                        or row[1] not in {"Form", "FormProperties", "FormAttribute"} and on_form is False
                    )]
                    if len(candidates) != 1:
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
        with temp_tree("unica-s047-") as raw:
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
        """
        rule: CTR.SOURCE.EMPTY-BRANCHES
        rule: INV.ANALYSIS.METADATA-DIAGNOSTIC-VERDICT
        rule: INV.APP.SUPPORT-STATE
        rule: INV.SOURCE.BORROWED-OBJECT-INHERITED-CONTENT
        rule: INV.SOURCE.EVENT-IMPLEMENTATION-PAIR
        rule: INV.SOURCE.FORM-EVENT-HANDLER-PARAMETERS
        rule: INV.SOURCE.MODULE-EVENT-STATE
        rule: INV.SOURCE.SUPPORT-SEPARATE-APPLY
        rule: INV.SOURCE.VENDOR-OBJECT-REMOVAL
        rule: INV.SOURCE.WRITABLE-PROFILE
        rule: INV.SURFACE.CHECK-INFERS-VALIDATORS
        """
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


def packed_json_strings(value, path=""):
    """Strings of a `view` answer that hold a JSON object or array.

    A composite value travels as nested JSON (#2031): a string the agent has
    to parse a second time costs tokens on every escaped quote."""
    if isinstance(value, dict):
        return [found for key, item in value.items() for found in packed_json_strings(item, f"{path}/{key}")]
    if isinstance(value, list):
        return [found for index, item in enumerate(value) for found in packed_json_strings(item, f"{path}/{index}")]
    if isinstance(value, str) and value.lstrip()[:1] in "{[":
        try:
            parsed = json.loads(value)
        except ValueError:
            return []
        if isinstance(parsed, (dict, list)):
            return [f"{path or '/'} packs JSON into a string: {value[:80]}"]
    return []


def call_step(server, step, arguments, label, *, delivery=False):
    if step.get("awaitTask"):
        return completed_delivery_call(server, step["tool"], arguments, label, retry_pending_index=False)
    if delivery:
        return completed_delivery_call(server, step["tool"], arguments, label)
    return server.call(step["tool"], arguments, label)


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
        prepare_source_workspace(workspace, SOURCE_WORKSPACE_PROFILES.get(workspace_relative, {}))
        state = home / "state"
        state.mkdir()
        environment = isolated_environment(plugin, state) if plugin else None
        if workspace_relative == HOST_CONTEXT_MISSING_WORKSPACE:
            environment = missing_host_context_environment(environment)
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
                        response = call_step(
                            server, step, arguments, label,
                            delivery=bool(plugin) or wanted == "tests/fixtures/acceptance/workspace-search-wide",
                        )
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
                    if step["tool"] == "unica.view":
                        failures.extend(packed_json_strings(structured))
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
    # Daemons outlive their servers by the idle grace with state files open;
    # the removal below waits them out instead of leaving the directory.
    raw = tempfile.mkdtemp(prefix="unica-acceptance-")
    try:
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
    finally:
        remove_tree(raw, 30)
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


def fault_cases(corpus, driver):
    selected = select_profile(corpus, "fault-injection")
    return {**selected, "scenarios": [
        scenario for scenario in selected["scenarios"] if scenario["driver"] == driver
    ]}


if __name__ == "__main__":
    unittest.main()
