"""Acceptance scenario followed by a BSL check on the real 1C platform.

A declaration in `tests/fixtures/acceptance/platform-checks/<scenario>.json`
names an acceptance scenario and a BSL module that judges its result inside
the platform:

    {
      "scenario": "S-…",
      "module": "verify.bsl",
      "client": "thick",
      "pass": "PASS: …",
      "negativeControl": "text of the expected FAIL"
    }

`module` is relative to the scenario's workspace and lies outside `src`, so
the scenario itself never runs it. The harness runs the scenario's wire
through MCP on a fresh copy of the fixture. Then, in the same session, it
places the module as the managed application module, creates an empty file
infobase from the resulting sources (`infobase.create`) and starts the client
with `launch`. The module receives a fresh empty result directory as its
launch parameter (`/C`), writes `result.txt` there and ends its own session.
The check passes only on a fresh `result.txt` whose single line equals `pass`
exactly.

`client` is `thin` (the default) or `thick`. The thin client is awaited by
`launch` itself (`waitForExit`). The thick client is what a check needs when
it reads configuration objects unavailable on a thin client — the
configuration's XDTO packages are absent from its `ФабрикаXDTO`. `launch`
does not await a thick client, so the harness waits for the fresh result
file the module writes before it ends its session.

`negativeControl`, when present, runs the same module on the pristine fixture
without the wire: the verdict must start with `FAIL: ` and contain this text.
It proves the module tells the scenario's outcome from the source it started
with, and not from a failure of its own.

The platform and v8-runner come through the product itself, so every step is
the one an agent takes. The declaration list is read from the directory: a
new check is one file, not an edit here.
"""
from __future__ import annotations

import json
import time
from pathlib import Path

from tests.ci.acceptance_controls import load_acceptance_corpus, load_corpus, tool_result

REPO = Path(__file__).resolve().parents[2]
CHECKS = REPO / "tests/fixtures/acceptance/platform-checks"
KEYS = {"scenario", "module", "client", "pass", "negativeControl"}
REQUIRED = {"scenario", "module", "pass"}
CLIENTS = {"thin", "thick"}
FAIL_PREFIX = "FAIL: "
# Создание файловой базы и загрузка конфигурации идут десятки секунд,
# клиент с модулем проверки — столько же.
OPERATION_TIMEOUT_SECONDS = 900
CLIENT_TIMEOUT_SECONDS = 600
RESULT_FILE = "result.txt"


class PlatformCheckError(AssertionError):
    """The platform step or its verdict did not confirm the scenario."""


def load_platform_checks(root: Path = REPO, corpus=None) -> list[dict]:
    """Declarations with their scenarios, validated against the corpus."""
    corpus = corpus if corpus is not None else load_acceptance_corpus()
    scenarios = {scenario["id"]: scenario for scenario in corpus["scenarios"]}
    directory = root / "tests/fixtures/acceptance/platform-checks"
    checks = []
    for path in sorted(directory.glob("*.json")):
        declaration = load_corpus(path)
        if not isinstance(declaration, dict) or not REQUIRED <= declaration.keys() <= KEYS:
            raise ValueError(f"{path.name}: keys must be {sorted(REQUIRED)} and optional client, negativeControl")
        if path.stem != declaration["scenario"]:
            raise ValueError(f"{path.name}: the file is named after its scenario")
        scenario = scenarios.get(declaration["scenario"])
        if scenario is None:
            raise ValueError(f"{path.name}: scenario {declaration['scenario']} is not in the corpus")
        if scenario.get("format"):
            raise ValueError(f"{path.name}: a format-axis scenario has no single source tree")
        if declaration.get("client", "thin") not in CLIENTS:
            raise ValueError(f"{path.name}: client is one of {sorted(CLIENTS)}")
        workspace = scenario.get("workspace", corpus["workspace"])
        module = root / workspace / declaration["module"]
        if not module.is_file() or module.suffix != ".bsl":
            raise ValueError(f"{path.name}: module {declaration['module']} is not a BSL file of the workspace")
        if Path(declaration["module"]).parts[0] == "src":
            raise ValueError(f"{path.name}: the module lies outside src so the scenario never runs it")
        for key in ("pass", "negativeControl"):
            value = declaration.get(key, "x")
            if not isinstance(value, str) or not value.strip() or "\n" in value:
                raise ValueError(f"{path.name}: {key} is one non-empty line")
        if declaration["pass"].startswith(FAIL_PREFIX):
            raise ValueError(f"{path.name}: pass cannot be a {FAIL_PREFIX!r} verdict")
        checks.append({**declaration, "workspace": workspace, "scenarioBody": scenario})
    return checks


def run_operation(server, op: str, args: dict, label: str) -> dict:
    """`unica.run` to its terminal result; a long operation is a Task to wait for."""
    deadline = time.monotonic() + OPERATION_TIMEOUT_SECONDS
    response = server.call("unica.run", {"op": op, "args": args, "dryRun": False}, label,
                           timeout=OPERATION_TIMEOUT_SECONDS)
    task_id = None
    while True:
        result = tool_result(response) or {}
        task = (result.get("data") or {}).get("task")
        if not (result.get("ok") is True and task and task.get("status") in {"queued", "working"}):
            if result.get("ok") is not True:
                raise PlatformCheckError(f"{label}: {json.dumps(response, ensure_ascii=False)[:4000]}")
            return result
        current = task.get("taskId")
        if not current or task_id not in (None, current):
            raise PlatformCheckError(f"{label}: Task identity is missing or changed")
        task_id = current
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise PlatformCheckError(f"{label}: the Task did not finish in {OPERATION_TIMEOUT_SECONDS}s")
        response = server.call("unica.task.result", {"taskId": task_id, "waitMs": 7000}, label,
                               timeout=remaining)


def configure_workspace(workspace: Path, build: str) -> None:
    """Pin the platform build and give the workspace its own file infobase."""
    project = workspace / "v8project.yaml"
    text = project.read_text(encoding="utf-8-sig")
    if "tools:" in text or "workPath:" in text:
        raise PlatformCheckError("the fixture already pins its tools; the harness would hide them")
    project.write_text(
        f"workPath: .work\n{text.rstrip()}\ntools:\n  platform:\n    version: '{build}'\n",
        encoding="utf-8",
    )
    (workspace / "v8project.local.yaml").write_text(
        f"infobases:\n  origin:\n    connection: '/F {workspace / 'ib'}'\n",
        encoding="utf-8",
    )


def read_verdict(path: Path) -> str:
    payload = path.read_bytes()
    for bom, encoding in ((b"\xef\xbb\xbf", "utf-8"), (b"\xff\xfe", "utf-16-le"), (b"\xfe\xff", "utf-16-be")):
        if payload.startswith(bom):
            return payload[len(bom):].decode(encoding).strip()
    return payload.decode("utf-8").strip()


def await_verdict(verdict: Path, label: str) -> None:
    """A thick client is not awaited by `launch`: the module's file is the signal."""
    deadline = time.monotonic() + CLIENT_TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        if verdict.is_file():
            size = verdict.stat().st_size
            # The platform writes the file in one call; a stable size means it is closed.
            time.sleep(1)
            if size and verdict.stat().st_size == size:
                return
        time.sleep(1)
    raise PlatformCheckError(
        f"{label}: the module wrote no {RESULT_FILE} in {CLIENT_TIMEOUT_SECONDS}s; "
        "a thick client may still be running")


def platform_verdict(server, check: dict, results: Path, label: str) -> str:
    """Load the workspace into a new infobase and let the module judge it."""
    workspace = Path(server.workspace)
    module = (REPO / check["workspace"] / check["module"]).read_bytes()
    (workspace / "src/Ext").mkdir(parents=True, exist_ok=True)
    (workspace / "src/Ext/ManagedApplicationModule.bsl").write_bytes(module)
    run_operation(server, "infobase.create", {}, f"{label} infobase.create")
    results.mkdir(parents=True)
    verdict = results / RESULT_FILE
    client = check.get("client", "thin")
    # The module appends file names to this directory as is.
    arguments = {"clientMode": client, "applicationParameter": str(results) + ("\\" if "\\" in str(results) else "/")}
    if client == "thin":
        arguments |= {"waitForExit": True, "waitTimeoutMs": CLIENT_TIMEOUT_SECONDS * 1000}
    started = time.time()
    run_operation(server, "launch", arguments, f"{label} launch")
    if client != "thin":
        await_verdict(verdict, label)
    if not verdict.is_file():
        raise PlatformCheckError(f"{label}: the module wrote no {RESULT_FILE}")
    if verdict.stat().st_mtime < started - 1:
        raise PlatformCheckError(f"{label}: {RESULT_FILE} is older than the client run")
    return read_verdict(verdict)


def platform_driver(check: dict, build: str, results: Path, *, negative: bool = False):
    """A `run_corpus` scenario driver that ends the scenario with the platform check."""
    def driver(server, scenario):
        configure_workspace(Path(server.workspace), build)
        label = f"{scenario['id']} platform"

        def finish(_verification):
            verdict = platform_verdict(server, check, results, label)
            if negative:
                if not (verdict.startswith(FAIL_PREFIX) and check["negativeControl"] in verdict):
                    raise PlatformCheckError(
                        f"{label}: expected a {FAIL_PREFIX!r} verdict with "
                        f"{check['negativeControl']!r}, got {verdict!r}")
            elif verdict != check["pass"]:
                raise PlatformCheckError(f"{label}: expected {check['pass']!r}, got {verdict!r}")
        return finish
    return driver


def negative_scenario(check: dict) -> dict:
    """The pristine fixture: the scenario's workspace without its wire."""
    return {**check["scenarioBody"], "id": f"{check['scenario']}-negative-control", "wire": []}
