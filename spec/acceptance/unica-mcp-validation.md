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
- A tool publishes only arguments its handler reads and a call can reach
  (INV-MCP-REACHABLE-ARGS). The eight typed readers `unica.cf.info`,
  `unica.role.info`, `unica.subsystem.info`, `unica.dcs.info`,
  `unica.form.info`, `unica.mxl.info`, `unica.cfe.diff` and `unica.meta.info`
  publish the exact sets pinned by
  `every_narrowed_reader_publishes_its_exact_argument_set`; none of them falls
  back to the shared native argument list, and `unica.mxl.info` addresses a
  template by `TemplatePath` alone (ADR-0048).

## Mandatory MCP Smoke

Use a temporary cache directory and call the stdio server. The run checks server
identity on the wire (INV-MCP-SERVER-NAME), the public tool namespace (INV-MCP-NAMESPACE), the
overridable volatile cache root (INV-CACHE-WORKSPACE-ROOT), and dry-run reporting without
written state (INV-CACHE-WRITE-FREE-PREVIEW):

The automated JSON-RPC smoke additionally executes the four Meta operations on
a real Platform XML fixture and rejects the three retired names as unknown
tools. This is the runtime acceptance for `INV-MCP-META-SURFACE`; the exact
registry and schema projection is owned by its named contract test.

The `unica.meta.info` response additionally satisfies
`INV-MCP-META-INFO-COVERAGE`: `kind` is paired with a mandatory typed `details`
variant for all 23 metadata kinds, read properties do not grant writer rights,
and malformed nested HTTP/WebService or newly routed child collections are
`null` with a diagnostic instead of looking complete. The tracked manifest
contains 11 edge fixtures plus independent property/collection route matrices;
multilingual declarations preserve `{language, content}` pairs. The
fixture/profile guard is exercised by:

```sh
cargo test -p unica-coder manifest_and_profile_cover_every_platform_gated_metadata_kind -- --test-threads=1
```

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

## Project Health Acceptance

This matrix exercises `INV-MCP-PROJECT-READINESS`,
`INV-SOURCE-ROOT-SEPARATION`, and `INV-SOURCE-PORTABLE-GIT`. Run it with:

```sh
cargo test -p unica-coder project_health -- --test-threads=1
cargo test -p unica-coder --test platform_project_health -- --test-threads=1
python3.12 -m unittest tests.ci.test_unica_mcp_smoke
```

| Scenario | Expected result | Executable evidence |
| --- | --- | --- |
| Workspace without Git | `ready=true`, `repositoryReady=false`; the call still succeeds | `project_status_without_git_separates_source_and_repository_readiness`, `test_project_status_publishes_typed_readiness_without_mutation` |
| Source set with `path: .` | `ready=false` and `source_set.root_is_workspace` | `project_status_reports_workspace_root_source_set_without_mutation` |
| Ignore rule missing, local-only, or from an untracked file | `repositoryReady=false`; only a tracked `.gitignore` proves portability | `project_health_git_info_exclude_is_local_only`, `project_health_git_untracked_gitignore_is_local_only`, `project_health_git_missing_ignore_rules_do_not_create_probe_files` |
| Tracked `ConfigDumpInfo.xml` | A staged runtime sidecar is rejected; a legitimate metadata descriptor is retained | `project_health_git_runtime_sidecar_and_legitimate_descriptor_are_distinct`, `project_health_parent_repository_reports_repository_relative_remediation` |
| XDTO `Ext/Package.bin` | It is treated as text despite a broad binary rule | `project_health_repository_policy_classifies_platform_xml_roles_exactly` |
| Text EOL | LF in the index and uniform LF or CRLF in the worktree pass; mixed EOL and lone CR fail | `project_health_repository_policy_accepts_tracked_portable_attributes`, `project_health_repository_policy_detects_mixed_worktree_eol` |
| Cancellation, truncated output, invalid UTF-8, or malformed NUL | The affected check never becomes a partial `passed` result | `managed_child_*`, `project_health_git_index_parser_rejects_malformed_nul_record`, `project_health_repository_policy_parsers_require_complete_nul_protocols` |
| Successful inspection | No project files, Git index, or hidden services are changed | `project_health_full_portable_repository_is_ready`, `project_health_linked_source_route_is_reported_without_following_it`, `test_project_status_publishes_typed_readiness_without_mutation` |

## Regression Tests

```sh
cargo fmt --all -- --check
cargo clippy --package unica-coder --all-targets -- -D warnings
cargo test --package unica-coder
python3.12 -m pip install -r tests/ci/requirements.txt
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
Skills appear under the plugin prefix, for example `/unica:meta-info`, and
public tools appear as `mcp__plugin_unica_unica__<tool>` with every character
outside `A-Za-z0-9_-` replaced by `_`.

For the development contour the same signal is checked without a marketplace:
Codex installs the local plugin, and Claude Code loads the directory directly
with `claude --plugin-dir ./plugins/unica`.

## Workspace Service Acceptance

This section exercises INV-APP-CODE-PROVIDER-BOUNDARY (provider-neutral
orchestration), INV-MCP-CODE-SEARCH-ROLES (public search semantics),
INV-APP-LAZY-HIDDEN-SERVICES (hidden, workspace-scoped services),
INV-SOURCE-SINGLE-RESOLVED-ROOT (source-root selection),
INV-CACHE-WORKTREE-ISOLATION (independent provider state),
INV-MCP-SDK-TRANSPORT and INV-MCP-BOUNDED-ADMISSION (transport ownership and
bounded admission), INV-CACHE-PERSISTED-STALENESS (live services are notified
by applied mutations), and INV-PLATFORM-NO-ORPHAN-PROCESSES (process-tree
ownership).

- `initialize`, `tools/list`, `project.status`, and `project.map` must not create
  `.build/unica/services`.
- `unica.code.search` returns fixed `semantic`, `symbol`, and `lexical` role
  sections in that order, with the current implementation named separately as
  `provider`, and may start the workspace service. A section exposes typed
  completeness, count relation, ranking, ordering, and a required nullable
  machine-readable termination reason whose code and retryability agree with
  the status. An RLM deadline exhausted while its index is still building is
  `timedOut` with retryable `dependencyPending/buildingIndex`; one failed or
  unavailable role remains visible while another successful or empty role
  makes the overall search successful. Cancellation returns no partial success.
- A search carrying an MCP progress token publishes a typed initial snapshot,
  role phase changes, a heartbeat at least every two seconds while work remains,
  and a terminal snapshot before its final response. A cold RLM index is waited
  for inside the same call and reported as `reconcilingSources`,
  `buildingIndex`, or `updatingIndex`; the client does not poll with another
  search.
- Search and diagnostics expose provider not-ready through the same
  `dependencyPending` vocabulary with `retryable`, `detailCode`, and available
  provider-supplied `retryAfterMs`/`state`; diagnostics also names
  `nextAction=status`, while a ready response takes no extra polling path.
- Analyzer-backed tools may create `.build/unica/services/<service-key>`.
- Repeated provider calls through one matching live service reuse independent
  `bsl-analyzer` and RLM transports. RLM reuses one logical `rlm_start` session
  until invalidation, expiry, shutdown, or source-root change and closes the
  old session with `rlm_end`.
- Another workspace, linked worktree, or source root must use another service,
  session, and index identity. No cross-chat reuse is part of the public
  contract, and no main-branch index is combined with a worktree delta.
- Stale or version-mismatched `service.json` records must be replaced.
- The canonical search call requires a non-empty `sourceSet` and optionally
  narrows it with `metadataPath`; a migration call may use only `sourceDir`.
  The selectors are mutually exclusive and logical resolution never falls back
  to the physical selector. Other path-based code readers preserve their
  existing source-root selection rules. Every effective physical root is
  normalized and rejected if it escapes the workspace
  (INV-SOURCE-SINGLE-RESOLVED-ROOT).
- RLM readiness records the complete trusted source revision. A warm unchanged
  APFS workspace does not rescan the corpus; an external write changes the
  generation before the next RLM admission, and a build that races a source
  write cannot publish `ready` for the captured revision
  (INV-CACHE-RLM-REVISION).
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
  Each code-intelligence provider separately admits at most 32 retained
  workers. A timed-out non-cooperative provider keeps its permit until it
  actually exits, and all retained handles share the same aggregate two-second
  shutdown grace as active tool calls.
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
cargo test -p unica-coder --test platform_code_intelligence -- --nocapture
```

Run the issue-89 test three consecutive times. Each run must finish within its test deadlines,
all recorded backend roots must end in `src/cf`, and no PID created by the
fixture may survive. On Windows, additionally inspect without terminating any
pre-existing user process:

```powershell
Get-Process rlm-bsl-index,bsl-analyzer -ErrorAction SilentlyContinue
```
