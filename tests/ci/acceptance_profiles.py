"""Closed execution profiles for the common acceptance corpus."""
from __future__ import annotations

import importlib.util
import json
import os
import platform
import shutil
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SYMBOL_WORKSPACE = "tests/fixtures/acceptance/workspace-symbol"
DIAGNOSTICS_WORKSPACE = "tests/fixtures/acceptance/workspace-diagnostics"
FAULT_CASES = {"empty", "invalid-event", "unknown-severity"}
DIAGNOSTIC_CASES = {"plain", "authors-toml", "authors-json", "diff-base", "precedence"}
DCS_WORKSPACE = "tests/fixtures/acceptance/workspace-dcs"
MXL_WORKSPACE = "tests/fixtures/acceptance/workspace-mxl"
AGENT_EVALUATIONS = {"dcs-contract": DCS_WORKSPACE, "mxl-contract": MXL_WORKSPACE}
SOURCE_WORKSPACES = {
    "tests/fixtures/acceptance/workspace-metadata-warning",
    DCS_WORKSPACE,
    MXL_WORKSPACE,
    "tests/fixtures/acceptance/workspace-code",
    "tests/fixtures/acceptance/workspace-resolve",
    "tests/fixtures/acceptance/workspace",
    "tests/fixtures/acceptance/workspace-format",
    "tests/fixtures/acceptance/workspace-bare",
}


def select_profile(corpus, profile):
    if profile not in {"source", "delivery", "agent-evaluation", "fault-injection"}:
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
        elif current == "delivery":
            if driver == "bsl-analyzer" and workspace == SYMBOL_WORKSPACE:
                if any(step["tool"] != "unica.search" or step["args"].get("role") != "symbol"
                       for step in scenario["wire"]):
                    raise ValueError(f"{scenario['id']}: analyzer driver supports symbol search only")
            elif driver == "bsl-analyzer-diagnostics" and workspace == DIAGNOSTICS_WORKSPACE:
                if scenario.get("evaluation") not in DIAGNOSTIC_CASES or any(
                    step["tool"] != "unica.check" or step["args"] != {"at": "main:CommonModule.Пример"}
                    for step in scenario["wire"]
                ):
                    raise ValueError(f"{scenario['id']}: invalid diagnostic case or wire")
            else:
                raise ValueError(f"{scenario['id']}: invalid delivery driver or fixture")
        elif current == "fault-injection":
            if driver != "analyzer-jsonl-fault" or workspace != DIAGNOSTICS_WORKSPACE or scenario.get("evaluation") not in FAULT_CASES or any(
                step["tool"] != "unica.check" or step["args"] != {"at": "main:CommonModule.Пример"}
                for step in scenario["wire"]
            ):
                raise ValueError(f"{scenario['id']}: invalid controlled JSONL driver, case or wire")
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
    builder.download(builder.release_asset_url(tool, asset), binary, timeout=60)
    builder.verify_asset_checksum(binary, asset, tool_name=tool["name"], target=target)
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
        result = (response or {}).get("result", {}).get("structuredContent") or {}
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
        if time.monotonic() >= deadline or poll == 19:
            raise TimeoutError(f"{label}: analyzer did not reach a terminal result")
        response = invoke(next_tool, next_args)
    raise AssertionError("unfinished delivery must fail")


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--stage-analyzer", type=Path, required=True)
    stage_analyzer(parser.parse_args().stage_analyzer)
