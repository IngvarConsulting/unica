# Unica MCP Acceptance

## Goal

Validate that the Unica plugin exposes one public MCP server, routes developer
workflows through that server, and keeps cache/state coordination inside the
orchestrator. One plugin directory serves both supported hosts, so host-facing
steps are run once per host (INV-PRODUCT-SINGLE-PLUGIN-TREE, ADR-0012).

Each section names the invariants it exercises. The normative wording of those
rules lives in [../architecture/invariants.md](../architecture/invariants.md)
and is not repeated here.

## Mandatory Local Contract

Run from the repository root:

```sh
python3.12 -m json.tool plugins/unica/.mcp.json >/dev/null
python3.12 -m json.tool plugins/unica/third-party/tools.lock.json >/dev/null
python3.12 -m json.tool plugins/unica/third-party/manifest.json >/dev/null
cargo run --quiet --bin unica -- --help
```

Expected:

- `.mcp.json` has exactly one key under `mcpServers`: `unica` (INV-MCP-SINGLE-ENTRY).
- `cargo run --quiet --bin unica -- --help` prints `unica <version>` and describes the stdio MCP
  orchestrator.
- Old adapter names are not public MCP registrations (INV-MCP-NO-ENGINE-SERVERS).
- Hidden workspace analyzer services are internal implementation details and do
  not add keys under `mcpServers` (INV-APP-LAZY-HIDDEN-SERVICES).
- Bundled-tool versions come from `plugins/unica/third-party/tools.lock.json`.
  Contract tests must load the locked entry and validate the corresponding
  artifact/interface; they must not hardcode a second `bsl-analyzer` version
  (INV-PRODUCT-TOOL-VERSION-SOURCE).
- Skill-local operation files do not exist. The only execution path is MCP
  `unica`; runtime shell/PowerShell wrappers are not shipped (INV-SKILL-NO-SCRIPT-ROUTE,
  INV-APP-NO-SCRIPT-BACKEND).

## Mandatory MCP Smoke

Use a temporary cache directory and call the stdio server. The run checks server
identity on the wire (INV-MCP-SERVER-NAME), the public tool namespace (INV-MCP-NAMESPACE), the
overridable volatile cache root (INV-CACHE-WORKSPACE-ROOT), and dry-run reporting without
written state (INV-CACHE-WRITE-FREE-PREVIEW):

```sh
python3.12 - <<'PY'
import json, os, subprocess, tempfile
from pathlib import Path

repo = Path.cwd()
with tempfile.TemporaryDirectory() as tmp:
    messages = [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "unica-acceptance", "version": "1"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
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

responses = {
    r["id"]: r for r in map(json.loads, result.stdout.splitlines()) if "id" in r
}
assert responses[1]["result"]["serverInfo"]["name"] == "unica"
tools = {tool["name"] for tool in responses[2]["result"]["tools"]}
assert "unica.project.status" in tools
assert "unica.form.edit" in tools
assert "unica.build.load" in tools
assert "unica.runtime.execute" in tools
assert "unica.standards.explain" in tools
assert all(not tool.startswith("bsl-") for tool in tools)
payload = json.loads(responses[3]["result"]["content"][0]["text"])
assert payload["cache"]["mode"] == "dry-run"
assert "FormChanged" in payload["cache"]["events"]
assert "metadata_graph" in payload["cache"]["invalidated"]
assert "lazy_rebuilt" in payload["cache"]
runtime_payload = json.loads(responses[4]["result"]["content"][0]["text"])
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

BSP parity fixtures preserve the exact committed fixture bytes under
`.gitattributes` `-text -whitespace`. Schema-v1 manifests may describe direct
harvests. The current schema-v2 test fixture is instead a deterministic,
declared projection from the harvested BSP `2.21` XML to the fixed `2.20`
profile:
`harvestedSize`/`harvestedSha256` identify the upstream bytes, while
`size`/`sha256` identify the projected committed bytes. It must not be described
as byte-identical to the upstream harvest. This immutable CI-fixture projection
is not a supported migration or downgrade operation and is never applied to
user source. `git diff --check` remains required for the rest of the tree.

## Skill Script Absence Acceptance

The migration to native `unica.*` handlers is complete: no skill ships or
references a skill-local Python/PowerShell operation file, and the runtime keeps
no script fallback (INV-SKILL-NO-SCRIPT-ROUTE, INV-APP-NO-SCRIPT-BACKEND). Use a check that avoids matching
package launchers:

```sh
rg -n 'powershell[.]exe|skills/.+[.]ps1|skills/.+[.]py' plugins/unica/skills
```

Expected: no matches anywhere under `plugins/unica/skills`. A match is a
regression against INV-SKILL-NO-SCRIPT-ROUTE and needs a superseding decision, not a tracked
migration task.

## Packaging Smoke

For the thin public package and its three runtime assets, the normal CI scripts
must satisfy (INV-PRODUCT-PACKAGE-PARITY, INV-PKG-THIN-PACKAGE, INV-PKG-VERIFIED-ATOMIC-INSTALL, INV-PKG-VERSION-LOCKSTEP):

- packaged `.mcp.json` exposes exactly `unica`;
- packaged `.mcp.json` uses only the command-scoped Git alias and target-neutral
  portable selector, and one launcher serves both hosts;
- the thin plugin has exactly three native bootstrap binaries and no full
  `bin/<target>` runtime;
- both host manifests are present at the same version, and the Claude manifest
  declares neither `skills` nor `mcpServers`, because that host discovers both
  by default;
- `runtime-manifest.json` pins the source commit, release tag, exact GitHub URLs,
  archive hashes, file hashes, and entrypoints for all targets;
- re-downloaded release archives exactly match the metadata and contain the
  generated `third-party/manifest.json` plus one target's runtime binaries;
- bootstrap `verify` completes MCP `initialize` and `tools/list` with the
  required stable public tools.

## Fresh Host Visibility

One plugin directory serves both hosts, so this section is run twice, once per
host (INV-PRODUCT-SINGLE-PLUGIN-TREE). On both hosts the acceptance signal is the same: a fresh
prompt showing Unica skills and only the public MCP server provided by the
plugin, not stale cached registrations (INV-MCP-NO-ENGINE-SERVERS, INV-MCP-SINGLE-ENTRY).

Codex: use a clean `CODEX_HOME`, add `IngvarConsulting/unica-marketplace` at
`main`, install `unica@unica`, and start a new Codex task.

Claude Code: uninstall `unica@unica`, remove the marketplace, and delete the
runtime cache under `${CLAUDE_PLUGIN_DATA}/runtimes` — it deliberately survives
a plugin update (INV-CACHE-RUNTIME-ROOT-ORDER). Then add the same marketplace, run
`claude plugin install unica@unica`, and start a new session or reload plugins.
Skills appear under the plugin prefix, for example `/unica:meta-validate`, and
public tools appear as `mcp__plugin_unica_unica__<tool>` with every character
outside `A-Za-z0-9_-` replaced by `_`.

For the development contour the same signal is checked without a marketplace:
Codex installs the local plugin, and Claude Code loads the directory directly
with `claude --plugin-dir ./plugins/unica`.

## Workspace Service Acceptance

This section exercises INV-APP-LAZY-HIDDEN-SERVICES (hidden, workspace-scoped services),
INV-SOURCE-SINGLE-RESOLVED-ROOT (source-root selection), INV-MCP-SDK-TRANSPORT and INV-MCP-BOUNDED-ADMISSION (transport
ownership and bounded admission), INV-CACHE-PERSISTED-STALENESS (live services are notified by
applied mutations), and INV-PLATFORM-NO-ORPHAN-PROCESSES (process-tree ownership).

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
  it escapes the workspace (INV-SOURCE-SINGLE-RESOLVED-ROOT).
- `project.status` and `project.map`, analyzer commands, RLM commands, and the
  workspace-service identity must agree on that effective source root.
- Analyzer and RLM work requests carry unique internal operation IDs. A public
  `notifications/cancelled` request must propagate to the matching operation.
  The cancelled public request itself gets no response, as the MCP
  specification prescribes (ADR-0013); the internal operation still observes
  cancellation exactly once.
- The public transport is the official Rust SDK (`rmcp`, ADR-0013,
  INV-MCP-SDK-TRANSPORT). It enforces the handshake: the first request must be a
  well-formed `initialize` (`ping` may precede it), and `protocolVersion` is
  negotiated with the client instead of being pinned.
- On EOF the SDK drains finishing calls (bounded at 5 seconds); the process
  then cancels all still-running domain operations, waits at most 2 seconds
  for them, and exits, closing stdout. Verify with
  `cargo test -p unica-coder eof_cancels_active_calls`.
- At most 32 `tools/call` workers are admitted; excess calls return `-32603`
  with `overloaded` without delaying `ping` or cancellation (INV-MCP-BOUNDED-ADMISSION).
  Public input line length is delegated to the SDK transport; the 8 MiB line
  bound below applies to the internal workspace-service protocol.
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
  process group. Other targets guarantee only immediate-child termination
  (INV-PLATFORM-NO-ORPHAN-PROCESSES).

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
