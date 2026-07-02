#!/usr/bin/env python3
"""Run Unica release assessment against a pinned 1c-syntax/ssl_3_2 ref."""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import time
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


SCHEMA_VERSION = 1
BSP_REPO = "https://github.com/1c-syntax/ssl_3_2"
BSP_REF = "3.2.1.446"
SOURCE_DIR = "src/cf"
EXPECTED_PUBLIC_TOOLS = {
    "unica.project.status",
    "unica.project.map",
    "unica.cf.info",
    "unica.cf.validate",
    "unica.code.diagnostics",
    "unica.code.search",
    "unica.code.grep",
    "unica.code.outline",
    "unica.meta.profile",
    "unica.standards.explain",
}


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def json_digest(value: Any) -> str:
    payload = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def read_json(path: Path, default: dict[str, Any] | None = None) -> dict[str, Any]:
    if not path.is_file():
        return default or {}
    return json.loads(path.read_text(encoding="utf-8-sig"))


def release_tag_from_env() -> str:
    ref = os.environ.get("GITHUB_REF_NAME") or os.environ.get("GITHUB_REF", "")
    if ref.startswith("refs/tags/"):
        return ref.removeprefix("refs/tags/")
    return ref or "manual"


def run_command(command: list[str], cwd: Path) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit(
            f"command failed with {result.returncode}: {' '.join(command)}\n{result.stdout}{result.stderr}"
        )
    return result.stdout.strip()


def download_bsp(work_dir: Path, *, repo: str = BSP_REPO, ref: str = BSP_REF) -> tuple[Path, str]:
    target = work_dir / "ssl_3_2"
    if target.exists():
        shutil.rmtree(target)
    work_dir.mkdir(parents=True, exist_ok=True)
    run_command(["git", "clone", "--depth", "1", "--branch", ref, repo, str(target)], work_dir)
    commit = run_command(["git", "rev-parse", "HEAD"], target)
    return target, commit


def safe_extract_tar(archive: Path, extract_dir: Path) -> None:
    root = extract_dir.resolve()
    with tarfile.open(archive) as tf:
        for member in tf.getmembers():
            target = (extract_dir / member.name).resolve()
            if not str(target).startswith(str(root) + os.sep) and target != root:
                raise SystemExit(f"refusing to extract path outside target directory: {member.name}")
        try:
            tf.extractall(extract_dir, filter="data")
        except TypeError:
            tf.extractall(extract_dir)


def safe_extract_zip(archive: Path, extract_dir: Path) -> None:
    root = extract_dir.resolve()
    with zipfile.ZipFile(archive) as zf:
        for name in zf.namelist():
            target = (extract_dir / name).resolve()
            if not str(target).startswith(str(root) + os.sep) and target != root:
                raise SystemExit(f"refusing to extract path outside target directory: {name}")
        zf.extractall(extract_dir)


def extract_marketplace_archive(archive: Path, extract_dir: Path) -> Path:
    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    extract_dir.mkdir(parents=True)

    if tarfile.is_tarfile(archive):
        safe_extract_tar(archive, extract_dir)
    elif zipfile.is_zipfile(archive):
        safe_extract_zip(archive, extract_dir)
    else:
        raise SystemExit(f"unsupported marketplace archive: {archive}")

    candidates = sorted(
        path
        for pattern in ("plugins/unica/bin/*/unica", "plugins/unica/bin/*/unica.exe")
        for path in extract_dir.rglob(pattern)
    )
    if not candidates:
        raise SystemExit(f"bundled unica binary not found after extracting {archive}")
    run_unica = candidates[0]
    run_unica.chmod(run_unica.stat().st_mode | 0o111)
    return run_unica


def plugin_root_for(run_unica: Path) -> Path:
    for parent in run_unica.parents:
        if (parent / ".codex-plugin" / "plugin.json").is_file():
            return parent
    return run_unica.parent


def unica_version(run_unica: Path) -> str:
    plugin_json = read_json(plugin_root_for(run_unica) / ".codex-plugin" / "plugin.json")
    return str(plugin_json.get("version", "unknown"))


def call_mcp(
    run_unica: Path,
    messages: list[dict[str, Any]],
    *,
    cwd: Path,
    cache_dir: Path,
    timeout_seconds: int,
) -> tuple[list[dict[str, Any]], int, str, str, int]:
    cache_dir.mkdir(parents=True, exist_ok=True)
    payload = "\n".join(json.dumps(message, ensure_ascii=False) for message in messages) + "\n"
    env = os.environ.copy()
    env["UNICA_CACHE_DIR"] = str(cache_dir)
    started = time.perf_counter()
    try:
        result = subprocess.run(
            [str(run_unica)],
            input=payload,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=cwd,
            env=env,
            timeout=timeout_seconds,
            check=False,
        )
        duration_ms = int((time.perf_counter() - started) * 1000)
    except subprocess.TimeoutExpired as error:
        duration_ms = int((time.perf_counter() - started) * 1000)
        return [], duration_ms, error.stdout or "", error.stderr or f"timed out after {timeout_seconds}s", 124

    responses: list[dict[str, Any]] = []
    for line in result.stdout.splitlines():
        if not line.strip():
            continue
        try:
            responses.append(json.loads(line))
        except json.JSONDecodeError:
            responses.append({"error": {"message": f"invalid JSON-RPC line: {line}"}})
    return responses, duration_ms, result.stdout, result.stderr, result.returncode


def tool_call_message(message_id: int, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": message_id,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    }


def parse_tool_payload(response: dict[str, Any]) -> tuple[dict[str, Any] | None, list[str]]:
    if "error" in response:
        error = response["error"]
        return None, [str(error.get("message", error))]
    try:
        text = response["result"]["content"][0]["text"]
    except (KeyError, IndexError, TypeError):
        return None, ["tool response does not contain text content"]
    try:
        payload = json.loads(text)
    except json.JSONDecodeError:
        payload = {"ok": True, "stdout": text, "warnings": [], "errors": [], "artifacts": []}
    if not isinstance(payload, dict):
        return None, ["tool text payload is not a JSON object"]
    return payload, []


def response_output_size(stdout: str, stderr: str, payload: dict[str, Any] | None) -> int:
    payload_size = 0 if payload is None else len(json.dumps(payload, ensure_ascii=False).encode("utf-8"))
    return len(stdout.encode("utf-8")) + len(stderr.encode("utf-8")) + payload_size


def parsed_stdout_payload(payload: dict[str, Any] | None) -> dict[str, Any]:
    if not payload:
        return {}
    stdout = payload.get("stdout")
    if not isinstance(stdout, str) or not stdout.strip():
        return {}
    try:
        parsed = json.loads(stdout)
    except json.JSONDecodeError:
        return {}
    return parsed if isinstance(parsed, dict) else {}


def project_source_sets(payload: dict[str, Any] | None) -> list[dict[str, Any]]:
    if not payload:
        return []
    for candidate in (payload, parsed_stdout_payload(payload)):
        source_sets = candidate.get("sourceSets")
        if source_sets is None:
            source_sets = candidate.get("source_sets")
        if isinstance(source_sets, list):
            return [item for item in source_sets if isinstance(item, dict)]
    return []


def scenario_result(
    *,
    scenario_id: str,
    title: str,
    tool: str,
    arguments: Any,
    status: str,
    duration_ms: int,
    blocking: bool,
    metrics: dict[str, Any] | None = None,
    errors: list[str] | None = None,
    artifacts: list[str] | None = None,
) -> dict[str, Any]:
    return {
        "id": scenario_id,
        "title": title,
        "tool": tool,
        "argumentsDigest": json_digest(arguments),
        "status": status,
        "durationMs": duration_ms,
        "blocking": blocking,
        "metrics": metrics or {},
        "errors": errors or [],
        "artifacts": artifacts or [],
    }


def run_tools_list_scenario(run_unica: Path, bsp_root: Path, cache_dir: Path, timeout_seconds: int) -> dict[str, Any]:
    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
    ]
    responses, duration_ms, stdout, stderr, returncode = call_mcp(
        run_unica,
        messages,
        cwd=bsp_root,
        cache_dir=cache_dir,
        timeout_seconds=timeout_seconds,
    )
    errors: list[str] = []
    tools: set[str] = set()
    if returncode != 0:
        errors.append(f"unica exited with {returncode}: {stderr.strip()}")
    if len(responses) != 2:
        errors.append(f"expected 2 JSON-RPC responses, got {len(responses)}")
    else:
        server_name = responses[0].get("result", {}).get("serverInfo", {}).get("name")
        if server_name != "unica":
            errors.append(f"expected serverInfo.name=unica, got {server_name!r}")
        tools = {tool.get("name", "") for tool in responses[1].get("result", {}).get("tools", [])}
        missing = sorted(EXPECTED_PUBLIC_TOOLS - tools)
        if missing:
            errors.append(f"missing expected public tools: {', '.join(missing)}")
    metrics = {
        "toolsCount": len(tools),
        "outputBytes": response_output_size(stdout, stderr, None),
    }
    return scenario_result(
        scenario_id="mcp-tools-list",
        title="MCP initialize and public tools list",
        tool="initialize+tools/list",
        arguments=messages,
        status="failed" if errors else "passed",
        duration_ms=duration_ms,
        blocking=True,
        metrics=metrics,
        errors=errors,
    )


def run_tool_scenario(
    run_unica: Path,
    *,
    bsp_root: Path,
    cache_dir: Path,
    scenario_id: str,
    title: str,
    tool: str,
    arguments: dict[str, Any],
    timeout_seconds: int,
    blocking: bool,
    require_payload_ok: bool,
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    args = dict(arguments)
    args.setdefault("cwd", str(bsp_root))
    message = tool_call_message(1, tool, args)
    responses, duration_ms, stdout, stderr, returncode = call_mcp(
        run_unica,
        [message],
        cwd=bsp_root,
        cache_dir=cache_dir,
        timeout_seconds=timeout_seconds,
    )
    errors: list[str] = []
    payload: dict[str, Any] | None = None
    if returncode != 0:
        errors.append(f"unica exited with {returncode}: {stderr.strip()}")
    if len(responses) != 1:
        errors.append(f"expected 1 JSON-RPC response, got {len(responses)}")
    elif not errors:
        payload, payload_errors = parse_tool_payload(responses[0])
        errors.extend(payload_errors)

    if payload is not None:
        payload_errors = [str(item) for item in payload.get("errors", []) if str(item).strip()]
        if payload.get("ok") is False and require_payload_ok:
            errors.extend(payload_errors or [str(payload.get("summary", f"{tool} reported ok=false"))])

    metrics = {
        "outputBytes": response_output_size(stdout, stderr, payload),
        "warningsCount": len(payload.get("warnings", [])) if payload else 0,
        "errorsCount": len(payload.get("errors", [])) if payload else len(errors),
    }
    source_sets = project_source_sets(payload)
    if source_sets:
        metrics["sourceSetsCount"] = len(source_sets)
    if payload and "cache" in payload:
        metrics["cache"] = payload["cache"]
    status = "failed" if errors else "passed"
    result = scenario_result(
        scenario_id=scenario_id,
        title=title,
        tool=tool,
        arguments=args,
        status=status,
        duration_ms=duration_ms,
        blocking=blocking,
        metrics=metrics,
        errors=errors,
        artifacts=[str(item) for item in payload.get("artifacts", [])] if payload else [],
    )
    return result, payload


def validate_project_map(scenario: dict[str, Any], payload: dict[str, Any] | None) -> None:
    if payload is None:
        return
    source_sets = project_source_sets(payload)
    if not source_sets:
        scenario["status"] = "failed"
        scenario["errors"].append("project map payload does not contain sourceSets")
        return
    found = any(
        item.get("path") == SOURCE_DIR and item.get("sourceFormat") in {"platform_xml", "PlatformXml"}
        for item in source_sets
        if isinstance(item, dict)
    )
    if not found:
        scenario["status"] = "failed"
        scenario["errors"].append(f"project map did not detect {SOURCE_DIR} as platform XML")


def relpath(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def first_existing(root: Path, patterns: list[str]) -> str | None:
    for pattern in patterns:
        matches = sorted(root.glob(pattern))
        if matches:
            return relpath(matches[0], root)
    return None


def template_kind(path: Path) -> str:
    text = path.read_text(encoding="utf-8", errors="ignore")[:8192]
    if "DataCompositionSchema" in text or "dataCompositionSchema" in text:
        return "skd"
    if "SpreadsheetDocument" in text or "spreadsheet" in text.lower():
        return "mxl"
    return "unknown"


def optional_sample_scenarios(bsp_root: Path) -> list[tuple[str, str, str, dict[str, Any], bool]]:
    scenarios: list[tuple[str, str, str, dict[str, Any], bool]] = []
    form = first_existing(bsp_root, [f"{SOURCE_DIR}/**/Forms/*/Ext/Form.xml"])
    if form:
        scenarios.append(("form-info-sample", "Sample managed form info", "unica.form.info", {"FormPath": form, "Limit": 80}, True))
        scenarios.append(
            ("form-validate-sample", "Sample managed form validation", "unica.form.validate", {"FormPath": form, "MaxErrors": 30}, False)
        )

    role = first_existing(bsp_root, [f"{SOURCE_DIR}/Roles/*/Ext/Rights.xml"])
    if role:
        scenarios.append(("role-info-sample", "Sample role info", "unica.role.info", {"RightsPath": role, "Limit": 80}, True))
        scenarios.append(
            ("role-validate-sample", "Sample role validation", "unica.role.validate", {"RightsPath": role, "MaxErrors": 30}, False)
        )

    templates = sorted(bsp_root.glob(f"{SOURCE_DIR}/**/Templates/*/Ext/Template.xml"))
    skd = next((path for path in templates if template_kind(path) == "skd"), None)
    mxl = next((path for path in templates if template_kind(path) == "mxl"), None)
    if skd:
        skd_rel = relpath(skd, bsp_root)
        scenarios.append(("skd-info-sample", "Sample SKD info", "unica.skd.info", {"TemplatePath": skd_rel, "Limit": 80}, True))
        scenarios.append(
            ("skd-validate-sample", "Sample SKD validation", "unica.skd.validate", {"TemplatePath": skd_rel, "MaxErrors": 30}, False)
        )
    if mxl:
        mxl_rel = relpath(mxl, bsp_root)
        scenarios.append(("mxl-info-sample", "Sample MXL info", "unica.mxl.info", {"TemplatePath": mxl_rel, "Limit": 80}, True))
        scenarios.append(
            ("mxl-validate-sample", "Sample MXL validation", "unica.mxl.validate", {"TemplatePath": mxl_rel, "MaxErrors": 30}, False)
        )
    return scenarios


def sample_bsl_path(bsp_root: Path) -> str | None:
    matches = sorted(bsp_root.glob(f"{SOURCE_DIR}/**/*.bsl"))
    return relpath(matches[0], bsp_root) if matches else None


def sample_bsl_search(bsp_root: Path) -> tuple[str, str] | None:
    for path in sorted(bsp_root.glob(f"{SOURCE_DIR}/**/*.bsl")):
        text = path.read_text(encoding="utf-8", errors="ignore")
        for query in ("Процедура", "Функция", "Экспорт"):
            if query in text:
                return relpath(path, bsp_root), query
    return None


def base_tool_scenarios(bsp_root: Path) -> list[tuple[str, str, str, dict[str, Any], bool, bool]]:
    bsl_search = sample_bsl_search(bsp_root)
    code_search_args = {"sourceDir": SOURCE_DIR, "query": "Процедура", "limit": 20}
    if bsl_search:
        bsl_path, query = bsl_search
        code_search_args = {"sourceDir": SOURCE_DIR, "path": bsl_path, "query": query, "limit": 20}

    scenarios: list[tuple[str, str, str, dict[str, Any], bool, bool]] = [
        ("project-status", "Workspace status", "unica.project.status", {}, True, True),
        ("project-map", "Workspace source-set map", "unica.project.map", {}, True, True),
        ("cf-info", "BSP Configuration.xml overview", "unica.cf.info", {"ConfigPath": SOURCE_DIR, "Mode": "brief", "Limit": 80}, True, True),
        ("cf-validate", "BSP Configuration.xml validation", "unica.cf.validate", {"ConfigPath": SOURCE_DIR, "MaxErrors": 50}, False, False),
        (
            "code-grep",
            "BSL git grep smoke",
            "unica.code.grep",
            code_search_args,
            True,
            True,
        ),
        (
            "code-diagnostics-workspace",
            "BSL diagnostics workspace read",
            "unica.code.diagnostics",
            {"sourceDir": SOURCE_DIR, "mode": "workspace", "limit": 100},
            False,
            False,
        ),
        (
            "code-search",
            "BSL indexed search smoke",
            "unica.code.search",
            code_search_args,
            False,
            True,
        ),
    ]
    bsl_path = sample_bsl_path(bsp_root)
    if bsl_path:
        scenarios.append(
            (
                "code-outline-sample",
                "Sample BSL module outline",
                "unica.code.outline",
                {"sourceDir": SOURCE_DIR, "path": bsl_path, "includeMethods": True},
                False,
                True,
            )
        )
    return scenarios


def extract_diagnostic_codes(payload: dict[str, Any] | None) -> list[str]:
    if not payload:
        return []
    codes: set[str] = set()
    diagnostics = payload.get("diagnostics")
    if isinstance(diagnostics, list):
        for item in diagnostics:
            if not isinstance(item, dict):
                continue
            for key in ("code", "id", "diagnostic", "diagnosticId"):
                value = item.get(key)
                if isinstance(value, str) and value:
                    codes.add(value)
    for text_key in ("stdout", "summary"):
        text = payload.get(text_key)
        if isinstance(text, str):
            for match in re.findall(r"\b[A-Z][A-Za-z0-9_]{4,}\b", text):
                codes.add(match)
    return sorted(codes)


def summarize_cache(cache_dir: Path) -> dict[str, Any]:
    if not cache_dir.exists():
        return {"exists": False, "files": 0, "bytes": 0}
    files = [path for path in cache_dir.rglob("*") if path.is_file()]
    return {
        "exists": True,
        "files": len(files),
        "bytes": sum(path.stat().st_size for path in files),
    }


def build_summary(scenarios: list[dict[str, Any]], diagnostic_codes: list[str], cache_dir: Path) -> dict[str, Any]:
    blocking_failures = sum(
        1 for scenario in scenarios if scenario["blocking"] and scenario["status"] == "failed"
    )
    total_duration = sum(int(scenario["durationMs"]) for scenario in scenarios)
    return {
        "status": "failed" if blocking_failures else "passed",
        "blockingFailures": blocking_failures,
        "qualityFindings": {
            "diagnosticCodes": diagnostic_codes,
            "nonBlockingFailures": sum(
                1 for scenario in scenarios if not scenario["blocking"] and scenario["status"] == "failed"
            ),
        },
        "performance": {
            "totalDurationMs": total_duration,
            "scenarioCount": len(scenarios),
            "cache": summarize_cache(cache_dir),
        },
    }


def environment_metadata(run_unica: Path) -> dict[str, Any]:
    return {
        "os": platform.platform(),
        "python": platform.python_version(),
        "machine": platform.machine(),
        "runUnica": str(run_unica),
        "generatedAt": utc_now(),
    }


def build_assessment_report(
    *,
    run_unica: Path,
    bsp_root: Path,
    cache_dir: Path,
    out_dir: Path,
    release_tag: str,
    github_run_id: str,
    candidate_package: str,
    bsp_commit: str,
    timeout_seconds: int,
    bsp_ref: str = BSP_REF,
) -> dict[str, Any]:
    scenarios: list[dict[str, Any]] = []
    diagnostic_codes: list[str] = []

    scenarios.append(run_tools_list_scenario(run_unica, bsp_root, cache_dir, timeout_seconds))

    for scenario_id, title, tool, arguments, blocking, require_payload_ok in base_tool_scenarios(bsp_root):
        scenario, payload = run_tool_scenario(
            run_unica,
            bsp_root=bsp_root,
            cache_dir=cache_dir,
            scenario_id=scenario_id,
            title=title,
            tool=tool,
            arguments=arguments,
            timeout_seconds=timeout_seconds,
            blocking=blocking,
            require_payload_ok=require_payload_ok,
        )
        if scenario_id == "project-map":
            validate_project_map(scenario, payload)
        scenarios.append(scenario)
        diagnostic_codes.extend(extract_diagnostic_codes(payload))

    for scenario_id, title, tool, arguments, require_payload_ok in optional_sample_scenarios(bsp_root):
        scenario, payload = run_tool_scenario(
            run_unica,
            bsp_root=bsp_root,
            cache_dir=cache_dir,
            scenario_id=scenario_id,
            title=title,
            tool=tool,
            arguments=arguments,
            timeout_seconds=timeout_seconds,
            blocking=False,
            require_payload_ok=require_payload_ok,
        )
        scenarios.append(scenario)
        diagnostic_codes.extend(extract_diagnostic_codes(payload))

    diagnostic_codes = sorted(set(diagnostic_codes))[:20]
    if diagnostic_codes:
        scenario, _payload = run_tool_scenario(
            run_unica,
            bsp_root=bsp_root,
            cache_dir=cache_dir,
            scenario_id="standards-explain-diagnostics",
            title="Explain top diagnostic codes through standards adapter",
            tool="unica.standards.explain",
            arguments={"codes": diagnostic_codes[:10]},
            timeout_seconds=timeout_seconds,
            blocking=False,
            require_payload_ok=True,
        )
        scenarios.append(scenario)

    description = read_json(bsp_root / "description.json")
    report = {
        "schemaVersion": SCHEMA_VERSION,
        "unicaVersion": unica_version(run_unica),
        "releaseTag": release_tag,
        "githubRunId": github_run_id,
        "candidatePackage": candidate_package,
        "bsp": {
            "repo": BSP_REPO,
            "ref": bsp_ref,
            "requestedRef": bsp_ref,
            "commit": bsp_commit,
            "descriptionVersion": description.get("Версия"),
            "descriptionDate": description.get("Дата"),
        },
        "environment": environment_metadata(run_unica),
        "scenarios": scenarios,
        "summary": build_summary(scenarios, diagnostic_codes, cache_dir),
    }
    write_report_files(report, out_dir)
    return report


def render_html(report: dict[str, Any]) -> str:
    status = html.escape(str(report["summary"]["status"]))
    release = html.escape(str(report["releaseTag"]))
    bsp_commit = html.escape(str(report["bsp"]["commit"]))
    blocking = html.escape(str(report["summary"]["blockingFailures"]))
    rows = []
    for scenario in report["scenarios"]:
        errors = "<br>".join(html.escape(error) for error in scenario.get("errors", []))
        rows.append(
            "<tr>"
            f"<td>{html.escape(str(scenario['id']))}</td>"
            f"<td>{html.escape(str(scenario['title']))}</td>"
            f"<td>{html.escape(str(scenario['tool']))}</td>"
            f"<td>{html.escape(str(scenario['status']))}</td>"
            f"<td>{html.escape(str(scenario['durationMs']))}</td>"
            f"<td>{html.escape(str(scenario['blocking']))}</td>"
            f"<td>{errors}</td>"
            "</tr>"
        )
    codes = ", ".join(html.escape(code) for code in report["summary"]["qualityFindings"].get("diagnosticCodes", []))
    return "\n".join(
        [
            "<!doctype html>",
            '<html lang="en">',
            "<head>",
            '<meta charset="utf-8">',
            "<title>Unica Release Assessment</title>",
            "<style>",
            "body{font-family:-apple-system,BlinkMacSystemFont,Segoe UI,sans-serif;margin:32px;line-height:1.45}",
            "table{border-collapse:collapse;width:100%;font-size:14px}",
            "th,td{border:1px solid #d0d7de;padding:6px 8px;text-align:left;vertical-align:top}",
            "th{background:#f6f8fa}",
            ".passed{color:#116329}.failed{color:#cf222e}",
            "</style>",
            "</head>",
            "<body>",
            f"<h1>Unica Release Assessment {release}</h1>",
            f'<p>Status: <strong class="{status}">{status}</strong></p>',
            f"<p>Blocking failures: {blocking}</p>",
            f"<p>BSP commit: <code>{bsp_commit}</code></p>",
            f"<p>Diagnostic codes: {codes or 'none'}</p>",
            "<h2>Scenarios</h2>",
            "<table>",
            "<thead><tr><th>ID</th><th>Title</th><th>Tool</th><th>Status</th><th>Duration ms</th><th>Blocking</th><th>Errors</th></tr></thead>",
            "<tbody>",
            *rows,
            "</tbody>",
            "</table>",
            '<p><a href="assessment.json">assessment.json</a> | <a href="assessment.ndjson">assessment.ndjson</a> | <a href="summary.md">summary.md</a></p>',
            "</body>",
            "</html>",
            "",
        ]
    )


def render_summary_markdown(report: dict[str, Any]) -> str:
    lines = [
        f"# Unica Release Assessment {report['releaseTag']}",
        "",
        f"- Status: `{report['summary']['status']}`",
        f"- Blocking failures: `{report['summary']['blockingFailures']}`",
        f"- BSP commit: `{report['bsp']['commit']}`",
        f"- BSP version: `{report['bsp'].get('descriptionVersion')}`",
        f"- Total duration: `{report['summary']['performance']['totalDurationMs']} ms`",
        "",
        "## Scenarios",
        "",
    ]
    for scenario in report["scenarios"]:
        lines.append(
            f"- `{scenario['status']}` `{scenario['id']}` ({scenario['durationMs']} ms, blocking={scenario['blocking']})"
        )
    lines.append("")
    return "\n".join(lines)


def write_report_files(report: dict[str, Any], out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "assessment.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    with (out_dir / "assessment.ndjson").open("w", encoding="utf-8") as stream:
        for scenario in report["scenarios"]:
            stream.write(json.dumps(scenario, ensure_ascii=False, sort_keys=True) + "\n")
    (out_dir / "index.html").write_text(render_html(report), encoding="utf-8")
    (out_dir / "summary.md").write_text(render_summary_markdown(report), encoding="utf-8")


def print_blocking_failure_summary(report: dict[str, Any]) -> None:
    failures = [
        scenario
        for scenario in report.get("scenarios", [])
        if scenario.get("blocking") and scenario.get("status") == "failed"
    ]
    if not failures:
        return
    print("blocking assessment failures:", file=sys.stderr)
    for scenario in failures:
        errors = "; ".join(str(error) for error in scenario.get("errors", [])) or "no error details"
        print(f"- {scenario.get('id')}: {errors}", file=sys.stderr)


def copytree_replace(source: Path, target: Path) -> None:
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target)


def copy_versioned_pages(report_dir: Path, pages_root: Path, release_tag: str) -> None:
    assessments = pages_root / "assessments"
    assessments.mkdir(parents=True, exist_ok=True)
    copytree_replace(report_dir, assessments / release_tag)
    copytree_replace(report_dir, assessments / "latest")
    index = assessments / "index.html"
    if not index.exists():
        index.write_text(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Unica Assessments</title></head>"
            "<body><h1>Unica Assessments</h1><p>Open a versioned assessment folder.</p></body></html>\n",
            encoding="utf-8",
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--package-archive", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, default=Path(".build/release-assessment/work"))
    parser.add_argument("--out-dir", type=Path, default=Path("dist/release-assessment"))
    parser.add_argument("--pages-root", type=Path)
    parser.add_argument("--release-tag", default=release_tag_from_env())
    parser.add_argument("--github-run-id", default=os.environ.get("GITHUB_RUN_ID", "local"))
    parser.add_argument("--bsp-ref", default=BSP_REF)
    parser.add_argument("--timeout-seconds", type=int, default=600)
    args = parser.parse_args()

    work_dir = args.work_dir.resolve()
    package_archive = args.package_archive.resolve()
    run_unica = extract_marketplace_archive(package_archive, work_dir / "marketplace")
    bsp_root, bsp_commit = download_bsp(work_dir / "bsp", ref=args.bsp_ref)
    report = build_assessment_report(
        run_unica=run_unica,
        bsp_root=bsp_root,
        cache_dir=work_dir / "cache",
        out_dir=args.out_dir,
        release_tag=args.release_tag,
        github_run_id=args.github_run_id,
        candidate_package=package_archive.name,
        bsp_commit=bsp_commit,
        bsp_ref=args.bsp_ref,
        timeout_seconds=args.timeout_seconds,
    )
    if args.pages_root:
        copy_versioned_pages(args.out_dir, args.pages_root, args.release_tag)
    if report["summary"]["blockingFailures"]:
        print_blocking_failure_summary(report)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
