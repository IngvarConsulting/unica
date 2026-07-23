# Command Selection

Use MCP `unica.runtime.execute` and choose `operation` by intent:

| Intent | Arguments |
|---|---|
| Missing project config | `operation=config-init`, optional `config`, `connection`, `format`, `builder` |
| Bind an existing external EPF config to a local infobase | `operation=config-init`, required `config`, `sourceSet`, `connection`; selected source-set must be `EXTERNAL_DATA_PROCESSORS` |
| Create runtime state | `operation=init` |
| Apply source changes to infobase | `operation=build`, optional `sourceSet`, `fullRebuild` |
| Bring infobase changes back to files | `operation=dump`, `mode=full`, optional `sourceSet`, `extension` |
| Convert Designer/EDT files | `operation=convert`, optional `sourceSet`, `output` |
| Export artifact | `operation=make`, required `output`, optional `sourceSet`, `extension` |
| Load artifact | `operation=load`, required `path`, optional `mode=load|merge`, `settings`, `extension` |
| Syntax check | `operation=syntax`, required `mode`, optional Designer flags or EDT `projects` |
| Tests | `operation=test`, required `testRunner`, optional YaXUnit `testScope`/`module`, `fullOutput`, VA filters |
| Client launch | `operation=launch`, required `clientMode`, optional MCP or direct launch flags |
| Bounded external EPF | `operation=launch`, `clientMode=thin`, required `execute`, `output`, `stderrOutput`, `waitForExit=true`, `waitTimeoutMs`; optional processing command in typed `c` |
| Extension properties | `operation=extensions`, optional `sourceSet` or `sourceSets` |
| Download runner tools | `operation=tools-download`, required `tool`, optional `sources`, `force` |

For branch switches, rebases, large object moves, or suspicious incremental state, use `operation=build` with `fullRebuild=true`.

For dumps, inspect the worktree before execution and compare the resulting diff after execution. Applied `mode=incremental|partial` is temporarily fail-closed until v8-runner publishes through shadow/staging with exact path/hash receipts; those modes remain available as `dryRun=true` previews.

Operation-specific guardrails:

- `build` does not accept `extension`; build an extension by selecting its configured `sourceSet`.
- `convert` does not accept ad hoc `path`, `format`, or `extension`; use configured source-sets.
- `load` does not support `mode=update`; use `mode=load` or `mode=merge` with `settings`.
- `test` uses `fullOutput=true` for v8-runner `--full`; it is not a build full rebuild.
- Bounded external EPF launch requires distinct paths: `output` is the platform `/Out` log, while `stderrOutput` captures stderr from the 1C client process. It rejects `/C`, `/Execute`, and `/Out` aliases in `rawKeys`; ordinary launch remains asynchronous.
- Put the external processor command-line payload in typed `c` (mapped to `/C`), not in `rawKeys`; Vanessa Automation commonly uses `StartFeaturePlayer;VAParams=<path>`.
- Prepare Vanessa Automation with `operation=tools-download`, `tool=vanessa`, then launch the default managed `build/tools/vanessa-automation-single.epf` or the effective `tools.va.epf_path` override.
- `tools-download` supports `sources=true` only for `tool=yaxunit` or `tool=client-mcp`.
