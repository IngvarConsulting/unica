# Unica MCP Acceptance

## Goal

Validate that the Unica plugin exposes one public MCP server, routes developer
workflows through that server, and keeps cache/state coordination inside the
orchestrator.

## Mandatory Local Contract

Run from the repository root:

```sh
python3.12 -m json.tool plugins/unica/.mcp.json >/dev/null
python3.12 -m json.tool plugins/unica/third-party/tools.lock.json >/dev/null
python3.12 -m json.tool plugins/unica/third-party/manifest.json >/dev/null
cargo run --quiet --bin unica -- --help
```

Expected:

- `.mcp.json` has exactly one key under `mcpServers`: `unica`.
- `cargo run --quiet --bin unica -- --help` prints `unica <version>` and describes the stdio MCP
  orchestrator.
- Old adapter names are not public MCP registrations.
- Hidden workspace analyzer services are internal implementation details and do
  not add keys under `mcpServers`.
- Skill-local operation files are not a target execution path. The target path is
  MCP `unica`; runtime shell/PowerShell wrappers are not shipped.

## Mandatory MCP Smoke

Use a temporary cache directory and call the stdio server:

```sh
python3.12 - <<'PY'
import json, os, subprocess, tempfile
from pathlib import Path

repo = Path.cwd()
with tempfile.TemporaryDirectory() as tmp:
    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "unica.form.edit",
                "arguments": {"dryRun": True, "cwd": tmp},
            },
        },
        {
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "unica.runtime.execute",
                "arguments": {"cwd": tmp, "operation": "dump"},
            },
        },
    ]
    env = os.environ.copy()
    env["UNICA_CACHE_DIR"] = str(Path(tmp) / "cache")
    result = subprocess.run(
        ["cargo", "run", "--quiet", "--bin", "unica", "--"],
        input="\n".join(json.dumps(message) for message in messages) + "\n",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=True,
        env=env,
    )

responses = [json.loads(line) for line in result.stdout.splitlines()]
assert responses[0]["result"]["serverInfo"]["name"] == "unica"
tools = {tool["name"] for tool in responses[1]["result"]["tools"]}
assert "unica.project.status" in tools
assert "unica.form.edit" in tools
assert "unica.build.load" in tools
assert "unica.runtime.execute" in tools
assert "unica.standards.explain" in tools
assert all(not tool.startswith("bsl-") for tool in tools)
payload = json.loads(responses[2]["result"]["content"][0]["text"])
assert payload["cache"]["mode"] == "dry-run"
assert "FormChanged" in payload["cache"]["events"]
assert "metadata_graph" in payload["cache"]["invalidated"]
assert "lazy_rebuilt" in payload["cache"]
runtime_payload = json.loads(responses[3]["result"]["content"][0]["text"])
assert runtime_payload["cache"]["mode"] == "dry-run"
assert "SourceSetChanged" in runtime_payload["cache"]["events"]
print("ok")
PY
```

## Regression Tests

```sh
cargo fmt --all -- --check
cargo clippy --package unica-coder --all-targets -- -D warnings
cargo test --package unica-coder
python3.12 -m unittest discover -s tests/ci
git diff --check
```

## Skill Script Removal Acceptance

For migrated skills, documentation and tests must reject workflow guidance that
points to skill-local Python/PowerShell operation files. Use a check that avoids
matching package launchers:

```sh
rg -n 'powershell[.]exe|skills/.+[.]ps1|skills/.+[.]py' plugins/unica/skills
```

Expected for fully migrated skills: no matches in their operation workflow
sections. Matches in not-yet-migrated skills are migration debt and must be
tracked in `spec/IMPLEMENTATION_TODO.md`.

## Packaging Smoke

For a local host-target package, build the tool bundle and package marketplace
with the normal CI scripts. A valid generated package must satisfy:

- packaged `.mcp.json` exposes exactly `unica`;
- packaged `.mcp.json` starts the bundled `bin/<target>/unica` binary directly
  with `cwd` set to the plugin root;
- generated `third-party/manifest.json` lists `unica` as a bundled tool and
  lists remote standards data only as an internal adapter.

## Fresh Codex Visibility

Use a clean `CODEX_HOME` when validating a packaged plugin. The acceptance
signal is a fresh prompt showing Unica skills and only the public MCP server
provided by the plugin, not stale cached MCP registrations.

## Workspace Service Acceptance

- `unica.code.grep` must not create `.build/unica/services`.
- Analyzer-backed tools may create `.build/unica/services/<service-key>`.
- Two sessions using the same workspace/source root should reuse a matching live
  service record.
- Another workspace or source root must use another service key.
- Stale or version-mismatched `service.json` records must be replaced.
