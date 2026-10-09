"""Real Codex execution, closed tool audit, source assertions and separate review.

The prepared packet contains tracked product sources and the current host binary;
it is a development packet, not evidence of a published release asset.
"""
from __future__ import annotations

import importlib.util
import json
import os
import platform
import shutil
import shlex
import signal
import subprocess
import sys
import time
import uuid
from pathlib import Path

from tests.ci.acceptance_profiles import REPO, host_target, isolated_environment

FIXTURE = REPO / "tests/fixtures/acceptance/agent-dcs"
XML = "src/cf/Reports/F05Report/Templates/F05Schema/Ext/Template.xml"
DCS_TEMPLATE = "cf:Report.F05Report.Template.F05Schema"
DISABLED = ("unified_exec", "plugins", "apps", "memories", "hooks",
            "multi_agent", "skill_search", "skill_mcp_dependency_install",
            "browser_use", "computer_use", "image_generation",
            "view_image", "tool_suggest", "shell_snapshot")


def packager():
    spec = importlib.util.spec_from_file_location("agent_packet", REPO / "scripts/ci/package-unica-plugin.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def prepare_packet(root):
    builder = packager()
    lock = json.loads((REPO / "plugins/unica/third-party/tools.lock.json").read_text())
    target = host_target(lock, platform.system(), platform.machine())
    packet = root / "plugin"
    builder.copy_tracked_plugin_source(REPO, REPO / "plugins/unica", packet)
    builder.assert_host_manifests_present(packet)
    if any((packet / "skills" / skill).exists() for skill in ("dcs-edit", "dcs-compile")):
        raise ValueError("evaluated packet still delivers retired DCS skills")
    binary = packet / "bin" / target / ("unica.exe" if os.name == "nt" else "unica")
    binary.parent.mkdir(parents=True)
    shutil.copy2(REPO / "target/debug" / binary.name, binary)
    for host in ("codex", "claude", "zcode"):
        builder.write_local_debug_mcp_launcher(packet, target, host=host)
        mcp = json.loads((packet / ".mcp.json").read_text())["mcpServers"]["unica"]
        expected = (f"./bin/{target}/{binary.name}" if host == "codex"
                    else f"${{CLAUDE_PLUGIN_ROOT}}/bin/{target}/{binary.name}")
        if mcp["command"] != expected or mcp["env"]["UNICA_HOST_CONTEXT_REQUIRED"] != "1":
            raise ValueError(f"invalid {host} launcher")
    builder.write_local_debug_mcp_launcher(packet, target, host="codex")
    return packet, binary, builder.package_tree_sha256(packet)


def disabled_skills():
    # Explicitly disable every locally discoverable prompt skill; plugins are
    # disabled separately. Never replace CODEX_HOME: auth remains available.
    roots = [Path.home() / ".agents/skills", Path.home() / ".codex/skills"]
    paths = sorted({path.parent.resolve() for root in roots if root.exists()
                    for path in root.glob("**/SKILL.md")})
    return paths


def codex_command(workspace, schema, final, skill_paths):
    command = ["codex", "exec", "--ignore-user-config", "--ignore-rules", "--ephemeral",
               "--json", "--skip-git-repo-check", "--sandbox", "read-only",
               "-C", str(workspace), "--output-schema", str(schema),
               "--output-last-message", str(final), "-c", 'web_search="disabled"',
               "-c", "tools.view_image=false"]
    command += ["-c", "project_doc_max_bytes=0"]
    command += ["-c", "features.code_mode=false", "-c", "features.code_mode_only=false"]
    for feature in DISABLED:
        command += ["--disable", feature]
    if skill_paths:
        value = "skills.config=[" + ",".join(
            "{path=" + json.dumps(str(path)) + ",enabled=false}" for path in skill_paths) + "]"
        command += ["-c", value]
    return command


def run_cli(command, prompt, environment, output, *, timeout=900):
    stdout = output.with_suffix(".jsonl")
    stderr = output.with_suffix(".stderr")
    with stdout.open("wb") as out, stderr.open("wb") as err:
        process = subprocess.Popen(command + ["-"], stdin=subprocess.PIPE, stdout=out,
                                   stderr=err, env=environment, start_new_session=True)
        try:
            process.communicate(prompt.encode(), timeout=timeout)
        finally:
            # Kill only the process group created by this invocation; child
            # MCP frontends must also stop on success and partial initialization.
            terminate_group(process.pid)
            process.wait(timeout=10)
        if process.returncode != 0:
            raise ValueError(f"Codex failed ({process.returncode}); see {stderr}")
    return [json.loads(line) for line in stdout.read_text().splitlines()], stdout, stderr


def terminate_group(pid):
    try:
        os.killpg(pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            os.killpg(pid, 0)
        except ProcessLookupError:
            return
        time.sleep(0.05)
    # The leader may already have exited. Its surviving children still belong
    # to this owned group and must not escape merely because poll() is terminal.
    try:
        os.killpg(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def allowed_help_command(command, help_path):
    parts = shlex.split(command)
    if len(parts) == 3 and parts[0] in {"/bin/zsh", "/bin/bash", "/bin/sh"} and parts[1] in {"-lc", "-c"}:
        parts = shlex.split(parts[2])
    return parts == ["cat", str(help_path)]


def audit_cli(events, allowed_tools, help_path):
    completed = []
    turns = 0
    help_reads = 0
    for event in events:
        kind = event.get("type")
        if kind in {"thread.started", "turn.started"}:
            continue
        if kind == "turn.completed":
            turns += 1
            continue
        if kind not in {"item.started", "item.updated", "item.completed"}:
            raise ValueError(f"unknown or failing Codex event: {kind}")
        item = event.get("item", {})
        item_kind = item.get("type")
        if item_kind in {"reasoning", "agent_message"}:
            continue
        if item_kind == "command_execution":
            if not allowed_help_command(item.get("command", ""), help_path):
                raise ValueError("agent shell command is outside the single prepared help file")
            if kind == "item.completed":
                if item.get("exit_code") != 0 or item.get("status") != "completed":
                    raise ValueError("prepared help read failed")
                help_reads += 1
            continue
        if item_kind != "mcp_tool_call" or item.get("server") != "unica" or item.get("tool") not in allowed_tools:
            raise ValueError(f"agent used a forbidden or unknown tool item: {item_kind}")
        if kind == "item.completed":
            refused = ((item.get("result") or {}).get("structured_content") or {}).get("ok") is False
            if item.get("error") or (item.get("status") != "completed" and not refused):
                raise ValueError("agent MCP transport failed")
            completed.append(item)
    if turns != 1 or not completed or help_reads < 1:
        raise ValueError("agent did not complete a tool-using turn with its reachable product help")
    return completed


def audit_mcp(records):
    requests = {}
    calls = []
    allowed = None
    for index, record in enumerate(records):
        payload = record["payload"]
        if record["direction"] == "request":
            if "id" in payload:
                requests[payload["id"]] = (index, record)
        else:
            request = requests.get(payload.get("id"))
            if request is None:
                continue  # a server notification has no matching request
            start, before = request
            message = before["payload"]
            if message["method"] == "tools/list":
                allowed = {tool["name"] for tool in payload["result"]["tools"]}
            if message["method"] == "tools/call":
                calls.append({"index": start, "done": index, "params": message["params"],
                              "result": (payload.get("result") or {}).get("structuredContent"),
                              "before": before["sourceSha256"], "after": record["sourceSha256"]})
    if not allowed:
        raise ValueError("no actual MCP tool inventory")
    calls.sort(key=lambda call: call["index"])
    raw_calls = calls
    calls = terminal_calls(calls)
    contracts = {}
    plans = {}
    executed = []
    for call in calls:
        tool = call["params"]["name"]
        args = call["params"]["arguments"]
        result = call["result"] or {}
        data = result.get("data") or {}
        if tool not in allowed:
            raise ValueError("call outside the recorded MCP inventory")
        if tool == "unica.view" and result.get("ok") is True:
            for entry in data.get("can") or []:
                if entry.get("implemented") is True and (entry.get("contract") or {}).get("argsSchema"):
                    contracts[(args.get("at"), entry["op"])] = call["done"]
        if tool == "unica.apply":
            if result.get("ok") is not True:
                if call["before"] != call["after"]:
                    raise ValueError("refused apply changed source bytes")
                continue  # a typed refusal may be corrected; it never proves an effect
            if data.get("mode") == "preview":
                if call["before"] != call["after"] or not isinstance(data.get("effects"), int) or data["effects"] < 1:
                    raise ValueError("preview changed bytes or did not plan a positive effect")
                operations = args.get("ops") or []
                if not operations:
                    raise ValueError("preview omitted operations")
                ops = []
                for operation in operations:
                    op = operation["op"]
                    target = operation.get("args", {}).get("at", args.get("at"))
                    values = operation.get("args", {}).get("values") or {}
                    # The public DCS contract also selects a dataset/variant
                    # by name. That effective node may differ from the common
                    # apply root, while still belonging to the same template.
                    if not isinstance(target, str) or not (target == DCS_TEMPLATE or target.startswith(DCS_TEMPLATE + ".")):
                        raise ValueError("operation target is outside the evaluated DCS template")
                    template = DCS_TEMPLATE
                    if isinstance(values.get("variant"), str) and values["variant"]:
                        target = template + ".Setting." + values["variant"]
                    elif isinstance(values.get("dataSet"), str) and values["dataSet"]:
                        target = template + ".DataSet." + values["dataSet"]
                    if contracts.get((target, op), call["index"]) >= call["index"]:
                        raise ValueError("detailed contract did not precede the apply target/op")
                    ops.append(op)
                token = data.get("executionToken")
                if not isinstance(token, str) or not token or token in plans:
                    raise ValueError("missing or repeated preview token")
                plans[token] = (ops, call["done"], call["after"], data["effects"])
            elif data.get("mode") == "published":
                plan = plans.pop(args.get("executionToken"), None)
                if not plan or plan[1] >= call["index"] or plan[2] != call["before"]:
                    raise ValueError("execute did not consume its observed preview")
                if data.get("effects") != plan[3] or call["before"] == call["after"]:
                    raise ValueError("execution did not publish its XML effect")
                executed.extend((op, call["done"]) for op in plan[0])
            else:
                raise ValueError("unobserved asynchronous apply")
    operations = {op for op, _ in executed}
    if (not {"field.add"} <= operations or not operations & {"query.set", "query.patch"}
            or not operations & {"structure.set", "structure.patch"} or plans):
        raise ValueError("agent did not publish the requested field, query and grouping effects")
    last = max(done for _, done in executed)
    if not any(call["index"] > last and call["params"]["name"] == "unica.check"
               and call["params"]["arguments"].get("at") == DCS_TEMPLATE
               and (call["result"] or {}).get("ok") is True
               and ((call["result"] or {}).get("data") or {}).get("status") == "passed"
               for call in calls):
        raise ValueError("agent did not check the final image")
    if not any(call["index"] > last and call["params"]["name"] == "unica.view"
               and (call["result"] or {}).get("ok") is True
               and (call["params"]["arguments"].get("at") == DCS_TEMPLATE
                    or str(call["params"]["arguments"].get("at", "")).startswith(DCS_TEMPLATE + "."))
               for call in calls):
        raise ValueError("agent did not reread the final image")
    return allowed, raw_calls


def terminal_calls(calls):
    pending = {}
    terminal = []
    for call in calls:
        name = call["params"]["name"]
        result = call["result"] or {}
        task = (result.get("data") or {}).get("task")
        if name == "unica.task.result":
            task_id = call["params"]["arguments"].get("taskId")
            original = pending.get(task_id)
            if original is None:
                raise ValueError("Task result has no observed originating call")
            if task:
                if task.get("taskId") != task_id or task.get("status") not in {"queued", "working", "completed"}:
                    raise ValueError("Task identity or state changed")
                continue
            pending.pop(task_id)
            terminal.append({**original, "done": call["done"], "after": call["after"], "result": call["result"]})
        elif name in {"unica.task.get", "unica.task.cancel"}:
            task_id = call["params"]["arguments"].get("taskId")
            if task_id not in pending or not task or task.get("taskId") != task_id:
                raise ValueError("Task observer changed or omitted the observed identity")
        elif task:
            task_id = task.get("taskId")
            if not isinstance(task_id, str) or not task_id or task_id in pending:
                raise ValueError("missing or duplicate originating Task identity")
            pending[task_id] = call
        elif name not in {"unica.task.get", "unica.task.cancel"}:
            terminal.append(call)
    if pending:
        raise ValueError("Task never reached an observed terminal result")
    return sorted(terminal, key=lambda call: call["index"])


def stop_owned_daemons(state, binary):
    for endpoint in state.glob("daemon-p5-*/endpoint.json"):
        pid = json.loads(endpoint.read_text())["pid"]
        def owned_alive():
            result = subprocess.run(["ps", "-p", str(pid), "-o", "args="], capture_output=True, text=True)
            if result.returncode != 0:
                return False
            parts = shlex.split(result.stdout.strip())
            expected = [str(binary), "--daemon", "--state-root", str(state)]
            if parts[:4] != expected:
                raise ValueError("daemon endpoint PID no longer belongs to this evaluation")
            return True
        if owned_alive():
            os.kill(pid, signal.SIGTERM)
            deadline = time.monotonic() + 5
            while owned_alive() and time.monotonic() < deadline:
                time.sleep(0.05)
            if owned_alive():
                os.kill(pid, signal.SIGKILL)
                deadline = time.monotonic() + 5
                while owned_alive() and time.monotonic() < deadline:
                    time.sleep(0.05)
                if owned_alive():
                    raise TimeoutError("owned evaluation daemon survived cleanup")


def evaluate(server, scenario, proof_root):
    proof = proof_root / (scenario["id"] + "-" + uuid.uuid4().hex)
    proof.mkdir(parents=True)
    (proof / "scenario.json").write_text(json.dumps(scenario, ensure_ascii=False, indent=2))
    state = proof / "state"
    state.mkdir()
    packet, binary, package_hash = prepare_packet(proof)
    source_before = {path.relative_to(server.workspace).as_posix(): packager().sha256(path)
                     for path in (server.workspace / "src").rglob("*") if path.is_file()}
    skills = disabled_skills()
    shutil.copy2(FIXTURE / "AGENTS.md", server.workspace / "AGENTS.md")
    for name in ("prompt.md", "AGENTS.md", "output.schema.json"):
        shutil.copy2(FIXTURE / name, proof / name)
    help_path = packet / "references/use-cases/reports-printing.md"
    agents = (proof / "AGENTS.md").read_text().replace("{{HELP_PATH}}", str(help_path))
    (proof / "AGENTS.md").write_text(agents)
    (server.workspace / "AGENTS.md").write_text(agents)
    environment = isolated_environment(packet, state)
    environment.update(UNICA_PROVIDER_STATE_DIR=str(state), UNICA_DAEMON_IDLE_GRACE_MS="5000",
                       UNICA_WORKSPACE_SERVICE_IDLE_SECS="1", UNICA_HOST_CONTEXT_REQUIRED="1")
    command = codex_command(server.workspace, proof / "output.schema.json", proof / "final.json", skills)
    server.input_schema("unica.view")
    tool_settings = "{" + ",".join(json.dumps(name) + '={approval_mode="approve"}' for name in server._schemas) + "}"
    command += ["-c", f"mcp_servers.unica.tools={tool_settings}"]
    recorder = REPO / "tests/agent_evaluation/mcp_recorder.py"
    arguments = [str(recorder), str(binary), str(proof / "mcp.jsonl"), str(server.workspace / XML)]
    for key, value in {"command": sys.executable, "args": arguments, "cwd": str(packet),
                       "env": {key: value for key, value in environment.items() if key.startswith("UNICA_")}}.items():
        # JSON is a valid TOML scalar/array. TOML dictionaries use '='.
        encoded = ("{" + ",".join(json.dumps(k) + "=" + json.dumps(v) for k, v in value.items()) + "}"
                   if isinstance(value, dict) else json.dumps(value))
        command += ["-c", f"mcp_servers.unica.{key}={encoded}"]
    manifest = {"packageSha256": package_hash, "codexVersion": subprocess.check_output(["codex", "--version"], text=True).strip(),
                "disabledFeatures": list(DISABLED), "disabledSkills": [str(path) for path in skills],
                "command": command, "files": {name: packager().sha256(proof / name) for name in
                                              ("prompt.md", "AGENTS.md", "output.schema.json", "scenario.json")},
                "modelSelection": "CLI default; exec JSON does not expose the actual model slug"}
    (proof / "manifest.json").write_text(json.dumps(manifest, indent=2))
    try:
        prompt = (proof / "AGENTS.md").read_text() + "\n\n" + (proof / "prompt.md").read_text()
        events, stdout, stderr = run_cli(command, prompt, environment, proof / "agent")
    finally:
        stop_owned_daemons(state, binary)
    # Keep the actual post-image even when a later audit fails, so debugging
    # the harness does not erase the configuration the agent really produced.
    shutil.copytree(server.workspace, proof / "workspace", ignore=shutil.ignore_patterns(".build", ".git"))
    records = [json.loads(line) for line in (proof / "mcp.jsonl").read_text().splitlines()]
    source_after = {path.relative_to(server.workspace).as_posix(): packager().sha256(path)
                    for path in (server.workspace / "src").rglob("*") if path.is_file()}
    changed = {path for path in source_before.keys() | source_after.keys()
               if source_before.get(path) != source_after.get(path)}
    if changed != {XML} or packager().package_tree_sha256(packet) != package_hash:
        raise ValueError("agent changed an unrequested source or its prepared packet")
    allowed, calls = audit_mcp(records)
    completed = audit_cli(events, allowed, help_path)
    if len(completed) != len(calls):
        raise ValueError("CLI and raw MCP transcripts disagree on tool calls")
    manifest["toolInventory"] = sorted(allowed)
    manifest["agentTranscriptSha256"] = packager().sha256(stdout)
    manifest["mcpTranscriptSha256"] = packager().sha256(proof / "mcp.jsonl")
    manifest["sourceBefore"] = source_before
    manifest["sourceAfter"] = source_after
    (proof / "manifest.json").write_text(json.dumps(manifest, indent=2))
    return proof


def independent_review(proof):
    # The evaluator gets business intent, the complete wire transcript and the
    # final source views. It has neither the agent's context nor mutation tools.
    schema = {"type": "object", "additionalProperties": False, "properties": {
        "accepted": {"type": "boolean"}, "reasons": {"type": "string"}},
        "required": ["accepted", "reasons"]}
    (proof / "review.schema.json").write_text(json.dumps(schema))
    intent = (proof / "prompt.md").read_text()
    evidence = {"manifest": json.loads((proof / "manifest.json").read_text()),
                "final": json.loads((proof / "final.json").read_text()),
                "mcp": [json.loads(line) for line in (proof / "mcp.jsonl").read_text().splitlines()],
                "verification": json.loads((proof / "verification.json").read_text())}
    prompt = ("Независимо проверь приёмку. Любое невыполненное требование означает accepted=false. "
              "Требования: три заданных изменения действительно опубликованы и прочитаны; запрос "
              "задаёт строку Agent1310 и числа 3/4 правильным полям; имя исходной группы сохранено; "
              "check проходит. Ответ честно объясняет, что создание template даёт только каркас "
              "без набора данных и полная новая схема с набором/запросом/полями не поддерживается "
              "текущими публичными операциями. Нельзя обещать её сборку или советовать обход XML. "
              "Приведи конкретные причины вердикта. Инструменты не используй.\nЗапрос:\n" + intent +
              "\nСвидетельства:\n" + json.dumps(evidence, ensure_ascii=False))
    (proof / "review-prompt.md").write_text(prompt)
    command = codex_command(proof, proof / "review.schema.json", proof / "review.json", disabled_skills())
    command += ["--disable", "shell_tool"]
    events, stdout, stderr = run_cli(command, prompt, dict(os.environ), proof / "reviewer", timeout=600)
    audit_review(events)
    verdict = json.loads((proof / "review.json").read_text())
    receipt = {"accepted": verdict.get("accepted") is True, "reasons": verdict.get("reasons"),
               "files": {name: packager().sha256(proof / name) for name in
                         ("agent.jsonl", "mcp.jsonl", "manifest.json", "final.json", "verification.json", "scenario.json",
                          "review-prompt.md", "review.schema.json", "reviewer.jsonl", "review.json")}}
    (proof / "receipt.json").write_text(json.dumps(receipt, ensure_ascii=False, indent=2))
    if receipt["accepted"] is not True or not receipt["reasons"]:
        raise AssertionError(f"independent evaluation refused: {receipt['reasons']}; proof: {proof}")


def audit_review(events):
    turns = 0
    for event in events:
        kind = event.get("type")
        if kind in {"thread.started", "turn.started"}:
            continue
        if kind == "turn.completed":
            turns += 1
            continue
        if kind not in {"item.started", "item.updated", "item.completed"}:
            raise ValueError("independent evaluator emitted a failing or unknown event")
        if event.get("item", {}).get("type") not in {"reasoning", "agent_message"}:
            raise ValueError("independent evaluator used a tool or failed")
    if turns != 1:
        raise ValueError("independent evaluator did not complete exactly one turn")
