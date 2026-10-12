"""Real Codex execution, closed tool audit, source assertions and separate review.

The prepared packet contains tracked product sources and the current host binary;
it is a development packet, not evidence of a published release asset.
"""
from __future__ import annotations

import importlib.util
import json
import os
import platform
import re
import shutil
import shlex
import signal
import subprocess
import sys
import time
import uuid
import xml.etree.ElementTree as ET
from pathlib import Path

from tests.ci.acceptance_profiles import REPO, host_target, isolated_environment

FIXTURE = REPO / "tests/fixtures/acceptance/agent-dcs"
XML = "src/cf/Reports/F05Report/Templates/F05Schema/Ext/Template.xml"
DCS_TEMPLATE = "cf:Report.F05Report.Template.F05Schema"
DCS_OWNER = "cf:Report.F05Report"
NEW_DCS_TEMPLATE = DCS_OWNER + ".Template.Agent1310Schema"
NEW_DCS_XML = "src/cf/Reports/F05Report/Templates/Agent1310Schema/Ext/Template.xml"
DCS_CHANGED_PATHS = {XML, "src/cf/Reports/F05Report.xml",
                     "src/cf/Reports/F05Report/Templates/Agent1310Schema.xml", NEW_DCS_XML}
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


def prepare_packet(root, source_binary):
    builder = packager()
    lock = json.loads((REPO / "plugins/unica/third-party/tools.lock.json").read_text())
    target = host_target(lock, platform.system(), platform.machine())
    packet = root / "plugin"
    builder.copy_tracked_plugin_source(REPO, REPO / "plugins/unica", packet)
    builder.assert_host_manifests_present(packet)
    if any((packet / "skills" / skill).exists() for skill in ("dcs-edit", "dcs-compile", "mxl-compile", "mxl-decompile", "mxl-info")):
        raise ValueError("evaluated packet still delivers retired workflow skills")
    binary = packet / "bin" / target / ("unica.exe" if os.name == "nt" else "unica")
    binary.parent.mkdir(parents=True)
    shutil.copy2(source_binary, binary)
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


def protected_plan_files(operations, roots):
    """Bounded source inputs and writable XML, derived from evaluated reports.

    Preview replies expose logical changed nodes, not retained physical reads.
    Observe the addressed template subtree plus each metadata owner and source
    map; creation additionally permits registration in its report descriptor.
    """
    exact = {"v8project.yaml"}
    prefixes = set()
    writable = set()
    for operation in operations:
        target = operation["args"]["at"]
        match = re.fullmatch(r"([^:]+):Report\.([^.]+)(?:\.Template\.([^.]+)(?:\..*)?)?", target)
        if not match or match[1] not in roots:
            raise ValueError("cannot bind the plan to recorded source files")
        root = roots[match[1]]
        report = f"{root}/Reports/{match[2]}"
        exact.update({f"{root}/Configuration.xml", report + ".xml"})
        if operation["op"] == "template.add":
            names = [item.get("name") for item in operation["args"].get("items", [])]
            if not names or any(not isinstance(name, str) or not re.fullmatch(r"[^./\\]+", name) for name in names):
                raise ValueError("creation plan omitted safe template names")
            writable.add(report + ".xml")
        else:
            if not match[3]:
                raise ValueError("plan operation omitted its template")
            names = [match[3]]
        for name in names:
            template = f"{report}/Templates/{name}"
            exact.add(template + ".xml")
            prefixes.add(template + "/")
            writable.add(template + "/Ext/Template.xml")
            if operation["op"] == "template.add":
                writable.add(template + ".xml")
    return exact, prefixes, writable


def protected_snapshot(files, scope):
    exact, prefixes, _ = scope
    return {**{path: files.get(path) for path in exact},
            **{path: digest for path, digest in files.items()
               if any(path.startswith(prefix) for prefix in prefixes)}}


def changed_files(before, after):
    return {path for path in before.keys() | after.keys() if before.get(path) != after.get(path)}


def audit_publication_files(publications):
    verified = []
    for call, plan in sorted(publications, key=lambda publication: publication[0]["done"]):
        scope = plan[4]
        if scope is None:
            continue  # legacy traces retain the stricter global preimage check
        before = dict(call["filesBefore"])
        for peer, peer_scope, transitions in verified:
            if not call["index"] < peer["index"] < peer["done"] < call["done"]:
                continue
            if any(path in scope[0] or any(path.startswith(prefix) for prefix in scope[1])
                   for path in transitions):
                raise ValueError("peer publication changed protected plan source files during execution")
            for path, (old, new) in transitions.items():
                if before.get(path) != old:
                    raise ValueError("peer publication source transitions do not match observed files")
                if new is None:
                    before.pop(path, None)
                else:
                    before[path] = new
        writes = changed_files(before, call["filesAfter"])
        if not writes or not writes <= scope[2]:
            raise ValueError("execution wrote outside its planned XML files or published no effect")
        transitions = {path: (before.get(path), call["filesAfter"].get(path)) for path in writes}
        verified.append((call, scope, transitions))


def audit_mcp(records, *, target_template=DCS_TEMPLATE, owner=None, required_operations=None,
              extra_templates=()):
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
                              "before": before["sourceSha256"], "after": record["sourceSha256"],
                              "filesBefore": before.get("sourceFilesSha256"),
                              "filesAfter": record.get("sourceFilesSha256"),
                              "rootsBefore": before.get("sourceSetRoots"),
                              "rootsAfter": record.get("sourceSetRoots")})
    if not allowed:
        raise ValueError("no actual MCP tool inventory")
    calls.sort(key=lambda call: call["index"])
    raw_calls = calls
    calls = terminal_calls(calls)
    contracts = {}
    plans = {}
    executed = []
    publications = []
    templates = (target_template, *extra_templates)
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
            files = call.get("filesBefore")
            after_files = call.get("filesAfter")
            if (files is None) != (after_files is None):
                raise ValueError("incomplete per-file source evidence")
            observed_write = call["before"] != call["after"] or (files is not None and files != after_files)
            if result.get("ok") is not True:
                if observed_write:
                    raise ValueError("refused apply changed source bytes")
                continue  # a typed refusal may be corrected; it never proves an effect
            if data.get("mode") == "preview":
                if observed_write or not isinstance(data.get("effects"), int) or data["effects"] < 1:
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
                    owning_template = next((template for template in templates
                                            if isinstance(target, str) and
                                            (target == template or target.startswith(template + "."))), None)
                    in_template = owning_template is not None
                    creation_owner = op == "template.add" and owner is not None and target == owner
                    if not (in_template or creation_owner):
                        raise ValueError("operation target is outside the evaluated template or its creation owner")
                    if creation_owner:
                        # Scope creation to the actual requested template, so a
                        # publication of a new schema cannot satisfy an effect
                        # or invalidate a completed verification of its peer.
                        names = {item.get("name") for item in operation.get("args", {}).get("items", [])}
                        evaluated_names = {template.rsplit(".Template.", 1)[-1] for template in templates}
                        if not names or not names <= evaluated_names:
                            raise ValueError("creation names are outside the evaluated templates")
                        owning_template = next(template for template in templates
                                               if template.rsplit(".Template.", 1)[-1] in names)
                    template = owning_template
                    if isinstance(values.get("variant"), str) and values["variant"]:
                        target = template + ".Setting." + values["variant"]
                    elif isinstance(values.get("dataSet"), str) and values["dataSet"]:
                        target = template + ".DataSet." + values["dataSet"]
                    if contracts.get((target, op), call["index"]) >= call["index"]:
                        raise ValueError("detailed contract did not precede the apply target/op")
                    ops.append((op, template))
                token = data.get("executionToken")
                if not isinstance(token, str) or not token or token in plans:
                    raise ValueError("missing or repeated preview token")
                scope = None
                snapshot = call["after"]
                if files is not None:
                    if not isinstance(call["rootsAfter"], dict) or call["rootsBefore"] != call["rootsAfter"]:
                        raise ValueError("source map evidence changed during preview")
                    scoped_operations = [{"op": operation["op"], "args": {**operation.get("args", {}),
                                          "at": operation.get("args", {}).get("at", args.get("at"))}}
                                         for operation in operations]
                    scope = protected_plan_files(scoped_operations, call["rootsAfter"])
                    snapshot = protected_snapshot(after_files, scope)
                plans[token] = (ops, call["done"], snapshot, data["effects"], scope, call["rootsAfter"])
            elif data.get("mode") == "published":
                plan = plans.pop(args.get("executionToken"), None)
                if not plan or plan[1] >= call["index"]:
                    raise ValueError("execute did not consume its observed preview")
                if plan[4] is not None:
                    if files is None or call["rootsBefore"] != plan[5] or call["rootsAfter"] != plan[5]:
                        raise ValueError("execution omitted or changed source map evidence")
                    if plan[2] != protected_snapshot(files, plan[4]):
                        raise ValueError("execute changed protected plan source files since its preview")
                elif plan[2] != call["before"]:
                    raise ValueError("execute did not consume its observed preview")
                if data.get("effects") != plan[3] or not observed_write:
                    raise ValueError("execution did not publish its XML effect")
                publications.append((call, plan))
                executed.extend((op, template, call["done"]) for op, template in plan[0])
            else:
                raise ValueError("unobserved asynchronous apply")
    audit_publication_files(publications)
    selected = [(op, done) for op, template, done in executed if template == target_template]
    operations = {op for op, _ in selected}
    # A preview may be abandoned or become stale after another publication.
    # Only consumed plans prove effects; every publication was checked above.
    if required_operations is None:
        if (not {"field.add"} <= operations or not operations & {"query.set", "query.patch"}
                or not operations & {"structure.set", "structure.patch"}):
            raise ValueError("agent did not publish the requested field, query and grouping effects")
    elif not required_operations <= operations:
        raise ValueError("agent did not publish all requested template operations")
    last = max(done for _, done in selected)
    if not any(call["index"] > last and call["params"]["name"] == "unica.check"
               and call["params"]["arguments"].get("at") == target_template
               and (call["result"] or {}).get("ok") is True
               and ((call["result"] or {}).get("data") or {}).get("status") == "passed"
               for call in calls):
        raise ValueError("agent did not check the final image")
    if not any(call["index"] > last and call["params"]["name"] == "unica.view"
               and (call["result"] or {}).get("ok") is True
               and (call["params"]["arguments"].get("at") == target_template
                    or str(call["params"]["arguments"].get("at", "")).startswith(target_template + "."))
               for call in calls):
        raise ValueError("agent did not reread the final image")
    return allowed, raw_calls


def audit_dcs_contract(records):
    """Each schema must have its own published effects and final verification."""
    allowed, calls = audit_mcp(records, owner=DCS_OWNER,
                               extra_templates=(NEW_DCS_TEMPLATE,))
    audit_mcp(records, target_template=NEW_DCS_TEMPLATE, owner=DCS_OWNER,
              extra_templates=(DCS_TEMPLATE,),
              required_operations={"template.add", "dataSource.add", "dataSet.add",
                                   "field.add", "variant.add", "selection.add"})
    return allowed, calls


def verify_dcs_images(workspace):
    """Observe XML independently of the agent's final claims and MCP projections."""
    schema_ns = "http://v8.1c.ru/8.1/data-composition-system/schema"
    settings_ns = "http://v8.1c.ru/8.1/data-composition-system/settings"
    core_ns = "http://v8.1c.ru/8.1/data/core"
    md_ns = "http://v8.1c.ru/8.3/MDClasses"
    xsi_ns = "http://www.w3.org/2001/XMLSchema-instance"
    s, t = "{" + schema_ns + "}", "{" + settings_ns + "}"

    def one(parent, path):
        nodes = parent.findall(path)
        if len(nodes) != 1:
            raise ValueError(f"expected exactly one XML node: {path}")
        return nodes[0]

    def query_constants(query, text, amount, added):
        for pattern in [rf'"{text}"\s+(?:КАК|AS)\s+Category\b',
                        rf'(?<![\w.]){amount}\s+(?:КАК|AS)\s+Amount\b',
                        rf'(?<![\w.]){added}\s+(?:КАК|AS)\s+Added\b']:
            if re.search(pattern, query, re.IGNORECASE) is None:
                raise ValueError("published query does not assign the requested constants to the fields")

    old = ET.parse(workspace / XML).getroot()
    old_data = one(old, s + "dataSet[" + s + "name='F05Data']")
    old_fields = [field.findtext(s + "dataPath") for field in old_data.findall(s + "field")]
    if old_fields != ["Category", "Amount", "Added"]:
        raise ValueError("existing schema field names changed or Added is missing")
    added_field = one(old_data, s + "field[" + s + "dataPath='Added']")
    if added_field.findtext(s + "title/{" + core_ns + "}item/{" + core_ns + "}content") != "Added":
        raise ValueError("existing schema Added title is wrong")
    old_query = one(old_data, s + "query").text or ""
    query_constants(old_query, "Agent1310", 3, 4)
    old_settings = one(old, s + "settingsVariant[" + t + "name='F05Variant']/" + t + "settings")
    old_group = one(old_settings, t + "item[" + t + "name='F05Group']")
    old_group_by = [node.text for node in old_group.findall(t + "groupItems/" + t + "item/" + t + "field")]
    if old_group_by != ["Amount"]:
        raise ValueError("existing named group was not patched to Amount")

    new = ET.parse(workspace / NEW_DCS_XML).getroot()
    if new.tag != s + "DataCompositionSchema" or "version" in new.attrib:
        raise ValueError("created schema has a wrong root or a version attribute")
    new_source = one(new, s + "dataSource[" + s + "name='AgentSource']")
    if new_source.findtext(s + "dataSourceType") != "Local":
        raise ValueError("new schema does not contain its requested Local source")
    new_data = one(new, s + "dataSet[" + s + "name='AgentData']")
    if new_data.attrib.get("{" + xsi_ns + "}type", "").rsplit(":", 1)[-1] != "DataSetQuery":
        raise ValueError("new AgentData is not a Query dataset")
    if new_data.findtext(s + "dataSource") != "AgentSource":
        raise ValueError("new dataset refers to a different data source")
    new_fields = [field.findtext(s + "dataPath") for field in new_data.findall(s + "field")]
    if len(new_fields) != 3 or set(new_fields) != {"Category", "Amount", "Added"}:
        raise ValueError("new schema does not contain exactly its three requested fields")
    for field in new_data.findall(s + "field"):
        if field.findtext(s + "field") != field.findtext(s + "dataPath") or field.find(s + "valueType") is not None:
            raise ValueError("new query field name/type does not follow the query field contract")
    new_query = one(new_data, s + "query").text or ""
    query_constants(new_query, "Agent1310New", 7, 8)
    new_settings = one(new, s + "settingsVariant[" + t + "name='AgentVariant']/" + t + "settings")
    new_group = one(new_settings, t + "item[" + t + "name='AgentGroup']")
    new_group_by = [node.text for node in new_group.findall(t + "groupItems/" + t + "item/" + t + "field")]
    if new_group_by != ["Category"]:
        raise ValueError("new AgentGroup does not group by Category")
    selected = [node.text for node in new_settings.findall(t + "selection/" + t + "item/" + t + "field")]
    if len(selected) != 3 or set(selected) != {"Category", "Amount", "Added"}:
        raise ValueError("new variant does not select exactly its three requested fields")
    owner = ET.parse(workspace / "src/cf/Reports/F05Report.xml").getroot()
    registered = [node.text for node in owner.findall("./{" + md_ns + "}Report/{" + md_ns + "}ChildObjects/{" + md_ns + "}Template")]
    if registered != ["F05Schema", "Agent1310Schema"]:
        raise ValueError("report template registration lost the original or added an unrequested template")
    descriptor = ET.parse(workspace / "src/cf/Reports/F05Report/Templates/Agent1310Schema.xml").getroot()
    if descriptor.findtext("./{" + md_ns + "}Template/{" + md_ns + "}Properties/{" + md_ns + "}TemplateType") != "DataCompositionSchema":
        raise ValueError("created template descriptor is not DataCompositionSchema")
    return {"existing": {"fields": old_fields, "query": old_query, "group": "F05Group", "groupBy": old_group_by},
            "created": {"template": NEW_DCS_TEMPLATE, "dataSource": "AgentSource", "dataSet": "AgentData",
                        "fields": new_fields, "query": new_query, "variant": "AgentVariant",
                        "group": "AgentGroup", "groupBy": new_group_by, "selection": selected},
            "registeredTemplates": registered}


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
            terminal.append({**original, "done": call["done"], "after": call["after"], "filesAfter": call.get("filesAfter"), "rootsAfter": call.get("rootsAfter"), "result": call["result"]})
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
        def owned_alive(require_owned=False):
            result = subprocess.run(["ps", "-p", str(pid), "-o", "stat=,args="], capture_output=True, text=True)
            if result.returncode != 0:
                return False
            observation = result.stdout.strip().split(None, 1)
            if not observation or observation[0].startswith("Z"):
                return False
            parts = shlex.split(observation[1] if len(observation) > 1 else "")
            expected = [str(binary), "--daemon", "--state-root", str(state)]
            if parts[:4] != expected:
                if require_owned:
                    raise ValueError("daemon endpoint PID no longer belongs to this evaluation")
                # After a verified signal, an unrelated process at the same PID
                # means our daemon exited. It must never receive the next signal.
                return False
            return True
        if owned_alive(require_owned=True):
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


def evaluate(server, scenario, proof_root, *, fixture=FIXTURE, source="src", changed_paths=None, auditor=None):
    is_dcs_contract = fixture == FIXTURE and auditor is None
    auditor = audit_dcs_contract if is_dcs_contract else (auditor or audit_mcp)
    proof = proof_root / (scenario["id"] + "-" + uuid.uuid4().hex)
    proof.mkdir(parents=True)
    (proof / "scenario.json").write_text(json.dumps(scenario, ensure_ascii=False, indent=2))
    state = proof / "state"
    state.mkdir()
    packet, binary, package_hash = prepare_packet(proof, server.binary)
    source_before = {path.relative_to(server.workspace).as_posix(): packager().sha256(path)
                     for path in (server.workspace / "src").rglob("*") if path.is_file()}
    skills = disabled_skills()
    shutil.copy2(fixture / "AGENTS.md", server.workspace / "AGENTS.md")
    for name in ("prompt.md", "AGENTS.md", "output.schema.json"):
        shutil.copy2(fixture / name, proof / name)
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
    arguments = [str(recorder), str(binary), str(proof / "mcp.jsonl"), str(server.workspace / source), str(server.workspace),
                 json.dumps({"cf": f"{source}/cf"})]
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
    expected_changes = changed_paths if changed_paths is not None else (DCS_CHANGED_PATHS if is_dcs_contract else {XML})
    if changed != expected_changes or packager().package_tree_sha256(packet) != package_hash:
        raise ValueError("agent changed an unrequested source or its prepared packet")
    allowed, calls = auditor(records)
    if is_dcs_contract:
        observations = verify_dcs_images(server.workspace)
        (proof / "file-observations.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2))
        manifest["fileObservationsSha256"] = packager().sha256(proof / "file-observations.json")
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


def independent_review(proof, *, rubric=None):
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
    if (proof / "file-observations.json").exists():
        evidence["fileObservations"] = json.loads((proof / "file-observations.json").read_text())
    default_rubric = ("Три заданных изменения действительно опубликованы и прочитаны; запрос "
              "задаёт строку Agent1310 и числа 3/4 правильным полям; имя исходной группы сохранено; "
              "check исходной схемы проходит. Дополнительно реально создан и зарегистрирован "
              "макет Agent1310Schema вида DataCompositionSchema. В нём Local источник AgentSource, "
              "Query набор AgentData с Category=строка Agent1310New, Amount=число 7, Added=число 8; "
              "объявлены эти три поля. Вариант AgentVariant выбирает эти поля и содержит именованную "
              "группировку AgentGroup по Category. Создание и наполнение выполнены публичными "
              "базовыми операциями, подробные контракты фактически выбранных операций прочитаны "
              "до их применения. Каждая публикация потребляет наблюдённый preview; preview и отказ "
              "не меняют исходники. Каждая схема перечитана и проверена после собственных последних "
              "изменений. Независимые fileObservations подтверждают оба постобраза и регистрацию. "
              "Существующая схема и её группа сохранены, нет незапрошенных изменений. Ответ "
              "сообщает выполненное создание, а не прежний отказ о невозможности; не предлагает "
              "ручную запись XML или полный JSON DSL. Агент не выдаёт физическое чтение драйвера "
              "за собственное чтение XML и не заявляет загрузку или выполнение отчёта в 1С без "
              "соответствующего протокола. Отдельное предупреждение об отсутствии запуска 1С "
              "не требуется.")
    prompt = ("Независимо проверь приёмку. Любое невыполненное требование означает accepted=false. "
              "Требования: " + (rubric if rubric is not None else default_rubric) +
              " Приведи конкретные причины вердикта. Инструменты не используй.\nЗапрос:\n" + intent +
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
    if (proof / "file-observations.json").exists():
        receipt["files"]["file-observations.json"] = packager().sha256(proof / "file-observations.json")
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
