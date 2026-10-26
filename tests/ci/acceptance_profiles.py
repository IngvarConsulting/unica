"""Closed execution profiles for the common acceptance corpus."""
from __future__ import annotations

import importlib.util
import json
import os
import platform
import shutil
import time
from pathlib import Path

from tests.ci.acceptance_controls import load_corpus, tool_result

REPO = Path(__file__).resolve().parents[2]
SYMBOL_WORKSPACE = "tests/fixtures/acceptance/workspace-symbol"
DIAGNOSTICS_WORKSPACE = "tests/fixtures/acceptance/workspace-diagnostics"
FAULT_CASES = {"empty", "invalid-event", "unknown-severity", "exit-failure", "missing-engine", "checksum-mismatch"}
DIAGNOSTIC_CASES = {"plain", "authors-toml", "authors-json", "diff-base", "precedence"}
# A call graph branch orders the missing analyzer in the same call; the
# delivery-failure driver stages a package that names it without a binary.
DELIVERY_FAILURE_METHOD = "main:CommonModule.Пример.Method.ОбработатьПример"
DELIVERY_FAILURE_ADDRESSES = {
    DELIVERY_FAILURE_METHOD,
    f"{DELIVERY_FAILURE_METHOD}.Caller",
    f"{DELIVERY_FAILURE_METHOD}.Callee",
}
DCS_WORKSPACE = "tests/fixtures/acceptance/workspace-dcs"
MXL_WORKSPACE = "tests/fixtures/acceptance/workspace-mxl"
EXTERNAL_WORKSPACE = "tests/fixtures/acceptance/workspace-external"
EXTERNAL_CHECK_ADDRESSES = {
    None,
    "epf:ExternalDataProcessor.Import.Module.Object",
    "epf:ExternalDataProcessor.Import.Form.Main.Module.Form",
    "erf:ExternalReport.Sales.Module.Object",
}
AGENT_EVALUATIONS = {"dcs-contract": DCS_WORKSPACE, "mxl-contract": MXL_WORKSPACE,
                     "props-contract": "tests/fixtures/acceptance/workspace",
                     "xdto-contract": "tests/fixtures/acceptance/workspace-xdto-contract"}
ACCEPTANCE_ROOT = REPO / "tests/fixtures/acceptance"
SOURCE_PROFILE_DIR = "source-profiles"


def load_source_workspaces(root: Path = ACCEPTANCE_ROOT) -> set[str]:
    """Explicit source-fixture registrations, one file per sibling workspace.

    Adding a fixture does not edit a shared Python set. Declarations stay
    outside the workspaces copied into the product by the acceptance runner.
    """
    root = Path(root)
    directory = root / SOURCE_PROFILE_DIR
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError(f"{directory}: source profiles need a regular directory")
    workspaces = set()
    for declaration in sorted(directory.iterdir()):
        if declaration.name.startswith("."):
            continue
        if (declaration.suffix != ".json" or declaration.stem in {".", ".."}
                or declaration.is_symlink() or not declaration.is_file()):
            raise ValueError(f"{declaration}: expected a regular <workspace>.json declaration")
        if load_corpus(declaration) != {"profile": "source"}:
            raise ValueError(f"{declaration}: expected exactly a source profile")
        workspace = root / declaration.stem
        if workspace.is_symlink() or not workspace.is_dir():
            raise ValueError(f"{declaration}: source fixture must be a regular sibling directory")
        workspaces.add(f"tests/fixtures/acceptance/{workspace.name}")
    return workspaces


SOURCE_WORKSPACES = load_source_workspaces()


def helper_invalidation_step(step):
    arguments = step["args"]
    if step["tool"] == "unica.search":
        return (set(arguments) == {"query", "role"}
                and isinstance(arguments["query"], str)
                and arguments["role"] == "symbol")
    if step["tool"] != "unica.apply":
        return False
    if arguments == {"executionToken": "$executionToken"}:
        return True
    if set(arguments) != {"at", "ops"} or arguments["at"] != "main:CommonModule.SearchProbe":
        return False
    operations = arguments["ops"]
    if not isinstance(operations, list) or len(operations) != 1:
        return False
    operation = operations[0]
    return (isinstance(operation, dict) and set(operation) == {"op", "args"}
            and operation["op"] == "code.replace"
            and isinstance(operation["args"], dict)
            and set(operation["args"]) == {"at", "text"}
            and operation["args"]["at"] == "main:CommonModule.SearchProbe.Body"
            and isinstance(operation["args"]["text"], str))


def select_profile(corpus, profile):
    if profile not in {"source", "delivery", "agent-evaluation", "fault-injection", "runtime-fault", "lsp"}:
        raise ValueError(f"profile {profile!r} has no executable driver")
    selected = []
    for scenario in corpus["scenarios"]:
        current = scenario.get("profile", "source")
        workspace = scenario.get("workspace", corpus["workspace"])
        driver = scenario.get("driver")
        if not scenario["wire"]:
            raise ValueError(f"{scenario['id']}: executable wire is empty")
        if current == "source":
            if "driver" in scenario or workspace not in SOURCE_WORKSPACES:
                raise ValueError(f"{scenario['id']}: invalid source driver or fixture")
            if any(step["tool"] in {"unica.run", "unica.task.get", "unica.task.result", "unica.task.cancel"}
                   for step in scenario["wire"]):
                raise ValueError(f"{scenario['id']}: runtime operations need an executable runtime driver")
        elif current == "runtime-fault":
            from tests.ci.acceptance_pull_validation import WORKSPACE, CASES
            if (driver != "pull-validation-fault" or workspace != WORKSPACE
                    or scenario.get("evaluation") not in CASES
                    or [step["args"] for step in scenario["wire"]] != [
                        {"op": "pull", "args": {"sourceSet": "main", "force": True}, "dryRun": True},
                        {"op": "pull", "args": {"sourceSet": "main", "force": True}, "dryRun": False}]
                    or any(step["tool"] != "unica.run" for step in scenario["wire"])):
                raise ValueError(f"{scenario['id']}: invalid runtime-fault fixture or pull wire")
        elif current == "delivery":
            if driver == "bsl-analyzer-external" and workspace == EXTERNAL_WORKSPACE:
                if "evaluation" in scenario or any(
                    step["tool"] != "unica.check"
                    or step["args"].get("at") not in EXTERNAL_CHECK_ADDRESSES
                    or step["args"] != ({} if step["args"].get("at") is None
                                        else {"at": step["args"]["at"]})
                    for step in scenario["wire"]
                ):
                    raise ValueError(f"{scenario['id']}: invalid external check or arguments")
            elif driver == "bsl-analyzer" and workspace == SYMBOL_WORKSPACE:
                if any(step["tool"] != "unica.search" or step["args"].get("role") != "symbol"
                       for step in scenario["wire"]):
                    raise ValueError(f"{scenario['id']}: analyzer driver supports symbol search only")
            elif driver == "bsl-analyzer-invalidation" and workspace == SYMBOL_WORKSPACE:
                if "evaluation" in scenario or not all(helper_invalidation_step(step) for step in scenario["wire"]):
                    raise ValueError(f"{scenario['id']}: invalid analyzer invalidation wire")
            elif driver == "bsl-analyzer-form-diagnostics":
                from tests.ci.form_runtime_diagnostics import valid_scenario
                if not valid_scenario(scenario):
                    raise ValueError(f"{scenario['id']}: invalid runtime form diagnostic case")
            elif driver == "bsl-analyzer-diagnostics" and workspace == DIAGNOSTICS_WORKSPACE:
                if scenario.get("evaluation") not in DIAGNOSTIC_CASES or any(
                    step["tool"] != "unica.check" or step["args"] != {"at": "main:CommonModule.Пример"}
                    for step in scenario["wire"]
                ):
                    raise ValueError(f"{scenario['id']}: invalid diagnostic case or wire")
            else:
                raise ValueError(f"{scenario['id']}: invalid delivery driver or fixture")
        elif current == "fault-injection":
            if driver == "analyzer-delivery-failure":
                if workspace != DIAGNOSTICS_WORKSPACE or "evaluation" in scenario or any(
                    step["tool"] != "unica.view"
                    or step["args"] != {"at": step["args"].get("at")}
                    or step["args"]["at"] not in DELIVERY_FAILURE_ADDRESSES
                    for step in scenario["wire"]
                ):
                    raise ValueError(f"{scenario['id']}: invalid analyzer delivery failure wire")
            elif driver != "analyzer-jsonl-fault" or workspace != DIAGNOSTICS_WORKSPACE or scenario.get("evaluation") not in FAULT_CASES or any(
                step["tool"] != "unica.check" or step["args"] != {"at": "main:CommonModule.Пример"}
                for step in scenario["wire"]
            ):
                raise ValueError(f"{scenario['id']}: invalid controlled JSONL driver, case or wire")
        elif current == "lsp":
            from tests.ci.acceptance_lsp import validate_scenario
            validate_scenario(scenario)
        elif current == "agent-evaluation":
            if driver != "codex" or AGENT_EVALUATIONS.get(scenario.get("evaluation")) != workspace:
                raise ValueError(f"{scenario['id']}: invalid agent driver, evaluation or fixture")
            if any(step["tool"] not in {"unica.view", "unica.check"} for step in scenario["wire"]):
                raise ValueError(f"{scenario['id']}: agent post-image wire must be read-only")
        else:
            raise ValueError(f"{scenario['id']}: profile {current!r} has no executable driver")
        if current == profile:
            selected.append(scenario)
    return {**corpus, "scenarios": selected}


def tools_builder():
    path = REPO / "scripts/ci/build-unica-tools.py"
    spec = importlib.util.spec_from_file_location("acceptance_tools_builder", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def host_target(lock, system, machine):
    matches = [name for name, target in lock["targets"].items()
               if target["hostSystem"] == system
               and machine.lower() in [value.lower() for value in target["hostMachines"]]]
    if len(matches) != 1:
        raise ValueError(f"no unique pinned analyzer target for {system}/{machine}")
    return matches[0]


def stage_analyzer(plugin, *, builder=None, system=None, machine=None):
    """Provision the exact published analyzer, without changing an installed plugin."""
    builder = builder or tools_builder()
    lock_path = REPO / "plugins/unica/third-party/tools.lock.json"
    lock = builder.load_lock(lock_path)
    target = host_target(lock, system or platform.system(), machine or platform.machine())
    config = lock["targets"][target]
    tool = next(tool for tool in lock["tools"] if tool["name"] == "bsl-analyzer")
    asset = tool["assets"][target]
    binary = plugin / "bin" / target / (tool["binaryName"] + config["exe"])
    # Три попытки по 25 с укладываются в 90 с, которые приёмка даёт подготовке.
    builder.fetch_verified_asset(builder.release_asset_url(tool, asset), binary, asset,
                                 tool_name=tool["name"], target=target, timeout=25)
    builder.set_file_mode(binary, executable=True)
    entry = builder.tool_entry(
        target=target, target_triple=config["targetTriple"], name=tool["name"],
        version=tool["version"], repository=tool["repository"], tag=tool["sourceTag"],
        commit=tool["sourceCommit"], license_id=tool["license"], binary=binary,
        relative_binary=binary.relative_to(plugin).as_posix(),
        delivered_binary=binary.relative_to(plugin).as_posix(), artifact="bsl-analyzer",
    )
    (plugin / "third-party").mkdir(parents=True)
    shutil.copyfile(lock_path, plugin / "third-party/tools.lock.json")
    (plugin / "third-party/manifest.json").write_text(json.dumps({
        "tools": [entry], "targetTriple": config["targetTriple"],
    }), encoding="utf-8")
    return plugin


def isolated_environment(plugin, state):
    environment = dict(os.environ)
    # These channels must not choose another project, runtime or analyzer.
    contract = json.loads((REPO / "tests/fixtures/mcp/host-workspace-context.json").read_text())
    for key in [*contract["projectEnvironment"], "UNICA_RUNTIME_MANIFEST",
                "UNICA_HOST_CONTEXT_REQUIRED", "UNICA_TEST_WORKSPACE_SERVICE_EXE",
                "EMBEDDING_URL", "EMBEDDING_API_KEY"]:
        environment.pop(key, None)
    environment["BSL_MCP_IDLE_TTL_SECS"] = "1"
    environment["BSL_MCP_ORPHAN_GRACE_SECS"] = "1"
    environment["UNICA_PLUGIN_ROOT"] = str(plugin)
    environment["UNICA_ARTIFACT_CACHE"] = str(state / "empty-artifact-cache")
    return environment


def completed_delivery_call(server, tool, arguments, label):
    """Observe Task identity; retry only the provider's typed pending index."""
    deadline = time.monotonic() + 120
    def invoke(next_tool, next_args):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(f"{label}: analyzer did not reach a terminal result")
        return server.call(next_tool, next_args, label, timeout=remaining)
    response = invoke(tool, arguments)
    task_id = None
    for poll in range(20):
        result = tool_result(response) or {}
        task = (result.get("data") or {}).get("task")
        if result.get("ok") is True and task:
            if task.get("status") not in {"queued", "working", "completed"}:
                raise ValueError(f"unexpected Task state: {task.get('status')}")
            current = task.get("taskId")
            if not isinstance(current, str) or not current or (task_id and task_id != current):
                raise ValueError("Task identity is missing or changed")
            task_id = current
            next_tool, next_args = "unica.task.result", {"taskId": task_id, "waitMs": 7000}
        else:
            sections = (result.get("data") or {}).get("matches") or []
            if not (result.get("ok") is False and len(sections) == 1
                    and sections[0].get("role") == "symbol"
                    and sections[0].get("provider") == "bsl-analyzer"
                    and (sections[0].get("termination") or {}).get("code") == "dependencyPending"
                    and (sections[0].get("termination") or {}).get("retryable") is True):
                return response
            task_id = None
            next_tool, next_args = tool, arguments
            # The provider promised a different answer later, not at once.
            time.sleep(.25)
        if time.monotonic() >= deadline or poll == 19:
            raise TimeoutError(f"{label}: analyzer did not reach a terminal result")
        response = invoke(next_tool, next_args)
    raise AssertionError("unfinished delivery must fail")


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--stage-analyzer", type=Path, required=True)
    stage_analyzer(parser.parse_args().stage_analyzer)
