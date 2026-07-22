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
- Bundled-tool versions come from `plugins/unica/third-party/tools.lock.json`.
  Contract tests must load the locked entry and validate the corresponding
  artifact/interface; they must not hardcode a second `bsl-analyzer` version.
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
assert "unica.project.discover" in tools
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

## Mandatory Extension-Point Preflight

The packaged plugin ships `extension-point-discovery` with implicit invocation
enabled. For planning or implementing changes to existing typical or supported
configurations, CFE, forms, documents, processors, handlers, or tabular
sections, the skill must make a valid task-only `unica.project.discover` call
before planning and before mutation or manual XML/BSL edits.

Acceptance inspects only `OperationResult.data.discovery`. The task-only UT 11.5
fixture must return `partial`, retain the exact
`Document.ПриобретениеТоваровУслуг.TabularSection.Серии`,
`DataProcessor.ПодборСерийВДокументы`, and
`DataProcessor.ПодборСерийВДокументы.Form.РегистрацияИПодборСерийПоОднойСтрокеТоваров`
candidates, report a blocking insufficient-coverage warning, and include
`bsl_index_missing`. Every retained candidate must carry a brief advisory
recommendation with typed evidence basis. Skill routing tests require evidence-ID/location
dereferencing, public read-only gap closure, stop-on-material-gap behavior, the
selection record, and the analysis-snapshot boundary. The snapshot is not
mutation authorization, a freshness guarantee, or a mutation receipt.

Validate the source skill with:

```sh
python3.12 -m unittest tests.ci.test_unica_skills tests.ci.test_skill_provenance
python3.12 "${CODEX_HOME:-$HOME/.codex}/skills/.system/skill-creator/scripts/quick_validate.py" plugins/unica/skills/extension-point-discovery
```

## Regression Tests

```sh
cargo fmt --all -- --check
cargo clippy --package unica-coder --all-targets -- -D warnings
cargo test --package unica-coder
python3.12 -m unittest discover -s tests/ci
git diff --check
```

BSP parity fixtures are intentionally byte-for-byte harvested. Their subtree is
marked `-text -whitespace` in `.gitattributes`; fixture integrity is enforced by
manifest `size`/`sha256`, while `git diff --check` remains required for the rest
of the tree.

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

For the thin public package and its three runtime assets, the normal CI scripts
must satisfy:

- packaged `.mcp.json` exposes exactly `unica`;
- packaged `.mcp.json` uses only the command-scoped Git alias and target-neutral
  portable selector;
- the thin plugin has exactly three native bootstrap binaries and no full
  `bin/<target>` runtime;
- `runtime-manifest.json` pins the source commit, release tag, exact GitHub URLs,
  archive hashes, file hashes, and entrypoints for all targets;
- re-downloaded release archives exactly match the metadata and contain the
  generated `third-party/manifest.json` plus one target's runtime binaries;
- bootstrap `verify` completes MCP `initialize`, `tools/list`, and the packaged
  native task-only `unica.project.discover` fixture with the required stable
  public tools and typed discovery result.

## Fresh Codex Visibility

Use a clean `CODEX_HOME`, add `IngvarConsulting/unica-marketplace` at `main`,
install `unica@unica`, and start a new Codex task. The acceptance signal is a
fresh prompt showing Unica skills and only the public MCP server provided by the
plugin, not stale cached registrations.

## Workspace Service Acceptance

- `unica.code.grep` must not create `.build/unica/services`.
- Analyzer-backed tools may create `.build/unica/services/<service-key>`.
- Two sessions using the same workspace/source root should reuse a matching live
  service record.
- Another workspace or source root must use another service key.
- Stale or version-mismatched `service.json` records must be replaced.
- With no `sourceDir`, a source set named `main` is the effective source root;
  otherwise the sole `CONFIGURATION` source set is used. Multiple configuration
  source sets without `main` must fail with `invalid_source_root:`. An explicit
  `sourceDir` is resolved relative to request `cwd`, normalized, and rejected if
  it escapes the workspace.
- `project.status` and `project.map`, analyzer commands, RLM commands, and the
  workspace-service identity must agree on that effective source root.
- Analyzer and RLM work requests carry unique internal operation IDs. A public
  `notifications/cancelled` request must propagate to the matching operation and
  return JSON-RPC error `-32800` exactly once.
- EOF gives accepted MCP workers 250 ms to publish, then cancels them and waits
  at most 2 seconds more. After that bounded deadline the publication-admission
  gate closes without waiting for generic writer I/O. A response not already
  admitted cannot begin I/O; an admitted arbitrary `Write` may complete after
  the injectable handler returns. The real stdio process then exits and closes
  stdout. Verify with `cargo test -p unica-coder mcp_dispatcher_close`.
- Public MCP JSONL lines are limited to 8 MiB and at most 32 `tools/call`
  workers are admitted. Oversized lines return `-32700`; excess calls return
  `-32603` with `overloaded` without delaying `ping` or cancellation.
- `ping`, cancellation, and shutdown must remain responsive while analyzer or
  RLM work is active. Cancelling one request must not require restarting the
  service before a later request succeeds.
- Internal request/response lines are limited to 8 MiB. At most 64 general
  handlers, 8 reserved control handlers, and 8 work workers may run. A bounded
  64-socket control classifier uses a 500 ms aggregate lifetime and a 64 KiB
  classification prefix when general handlers are full. Classified work then
  returns `workspace service overloaded: general connection handlers are
  saturated`; unclassified overflow is closed. A complete `ping`, `cancel`, or
  `shutdown` must still complete through the reserved path.
- Request-header parsing has one 5-second aggregate deadline from accept. Reads
  poll in at most 100 ms slices and slow-drip bytes do not renew that deadline.
- Work and ordinary `Ping`, `Invalidate`, and `Shutdown` requests have one
  120-second overall deadline starting before connect. Control kinds have a
  500 ms connect cap; connect, write, flush, and read consume the remaining
  overall budget.
  Reads poll at 100 ms intervals, and cancellation takes precedence over timeout,
  EOF, protocol, and successful process-exit races. A best-effort `Cancel` has a
  separate 500 ms aggregate budget for connect, write, and flush and does not
  read a response.
  Verify with `cargo test -p unica-coder cancellable_connector`.
- Shutdown and client disconnect cancel owned operations and boundedly clean up
  their child process trees. On Windows this guarantee is implemented by
  suspended start followed by Job Object assignment; on Unix by a dedicated
  process group. Other targets guarantee only immediate-child termination.

The issue-89 end-to-end regression exercises a workspace with `main` and
`TESTS` source sets, concurrent analyzer/RLM calls, cancellation, ping, a
subsequent successful request, and descendant cleanup:

```sh
cargo test -p unica-coder --test issue_89_workspace_service -- --nocapture
```

Run it three consecutive times. Each run must finish within its test deadlines,
all recorded backend roots must end in `src/cf`, and no PID created by the
fixture may survive. On Windows, additionally inspect without terminating any
pre-existing user process:

```powershell
Get-Process rlm-bsl-index,bsl-analyzer -ErrorAction SilentlyContinue
```
