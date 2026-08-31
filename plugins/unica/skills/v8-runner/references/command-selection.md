# Command Selection

Use MCP `unica.runtime.execute` and choose `operation` by intent:

- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

Every classified operation runs when the user asks for it; the result names
the risk it carries:

| Risk code in the applied result | Operations |
|---|---|
| `runtime_risk_critical_non_abortable` | `init`, `build`, `load`, `test`, `extensions` |
| `runtime_risk_publication_without_bounded_recovery` | `config-init`, `dump`, `convert`, `make`, `tools-download` |
| `runtime_risk_unproven_process_ownership` | `syntax` with `mode=designer-config`, `designer-modules` or `edt`; `launch` with `waitForExit=true` |
| `runtime_risk_detached_child` | `launch` without `waitForExit` |

`syntax` in any other mode stays unclassified and still refuses
`runtime_operation_unbounded`. Every JSON example below keeps `dryRun: true`:
the examples are previews, the applied mode is the same call with
`dryRun: false`.

| Intent | Arguments |
|---|---|
| Preview a missing project config | `operation=config-init`, optional `config`, `connection`, `format`, `builder`, `dryRun=true` |
| Preview binding an existing external EPF config to a local infobase | `operation=config-init`, required `config`, `sourceSet`, `connection`, `dryRun=true`; selected source-set must be `EXTERNAL_DATA_PROCESSORS` |
| Preview runtime state creation | `operation=init`, `dryRun=true` |
| Preview applying source changes to infobase | `operation=build`, optional `sourceSet`, `fullRebuild`, `dryRun=true` |
| Preview bringing infobase changes back to files | `operation=dump`, `mode=full`, optional `sourceSet`, `extension`, `dryRun=true`; applied post-run validation/publication has no proved receipt bound |
| Preview Designer/EDT conversion | `operation=convert`, optional `sourceSet`, `output`, `dryRun=true` |
| Preview artifact export | `operation=make`, required `output`, optional `sourceSet`, `extension`, `dryRun=true` |
| Preview artifact load | `operation=load`, required `path`, optional `mode=load` or `mode=merge`, `settings`, `extension`, `dryRun=true` |
| Preview syntax check | `operation=syntax`, required `mode`, `dryRun=true`; EDT optionally accepts `projects` |
| Preview tests | `operation=test`, required `testRunner`, optional YaXUnit `testScope`/`module`, `fullOutput`, VA filters, `dryRun=true` |
| Preview detached client launch | `operation=launch`, required `clientMode`, optional MCP or direct launch flags, `dryRun=true` |
| Preview bounded external EPF | `operation=launch`, `clientMode=thin`, required `execute`, `output`, `stderrOutput`, `waitForExit=true`, `waitTimeoutMs`, `dryRun=true`; optional processing command in typed `c` |
| Preview extension properties | `operation=extensions`, optional `sourceSet` or `sourceSets`, `dryRun=true` |
| Preview runner-tool download | `operation=tools-download`, required `tool`, optional `sources`, `force`, `dryRun=true` |

For branch switches, rebases, large object moves, or suspicious incremental
state, preview `operation=build` with `fullRebuild=true`; the applied run carries
`runtime_risk_critical_non_abortable`, so cancellation is deferred until the
uninterruptible phase ends.

For dumps, inspect the worktree before preview. Every applied mode carries
`runtime_risk_publication_without_bounded_recovery`: even the Unica-owned
private-stage full dump has post-run
validation/publication without a proved upper bound for the terminal receipt.
Incomplete modes additionally require exact path/hash receipts and
divergence-safe merge.

Operation-specific guardrails:

- `build` does not accept `extension`; build an extension by selecting its configured `sourceSet`.
- `convert` does not accept ad hoc `path`, `format`, or `extension`; use configured source-sets.
- Applied `convert` carries `runtime_risk_publication_without_bounded_recovery` because it can publish Designer XML outside the verified dump boundary.
- Applied `config-init`, `make`, and `tools-download` write persistent state without a bounded rollback contract; the result says so, and an interrupted run may leave partial output.
- Every `syntax` mode carries unproved process ownership: EDT may use an interactive session, while Designer may create a separately grouped 1C process whose cleanup is not proved for every runner failure path.
- Do not pass `DumpConfigToFiles` or `LoadConfigFromFiles` through Designer `rawKeys`; Unica rejects those bypasses.
- `load` does not support `mode=update`; use `mode=load` or `mode=merge` with `settings`.
- `test` uses `fullOutput=true` for v8-runner `--full`; it is not a build full rebuild.
- A bounded external EPF preview requires distinct paths: `output` is the platform `/Out` log, while `stderrOutput` captures stderr from the 1C client process. It rejects `/C`, `/Execute`, and `/Out` aliases in `rawKeys`; an applied launch runs and carries `runtime_risk_unproven_process_ownership` with `waitForExit=true`, or `runtime_risk_detached_child` without it.
- Put the external processor command-line payload in typed `c` (mapped to `/C`), not in `rawKeys`; Vanessa Automation commonly uses `StartFeaturePlayer;VAParams=<path>`.
- Preview Vanessa Automation preparation with `operation=tools-download`, `tool=vanessa`, then launch only an already existing default managed `build/tools/vanessa-automation-single.epf` or the effective `tools.va.epf_path` override.
- `tools-download` supports `sources=true` only for `tool=yaxunit` or `tool=client-mcp`, and it replaces the prebuilt release artifact rather than adding to it: the runner switches to `mode: sources`. What that yields differs by tool — `client-mcp` gets an EDT tree that only `1cedtcli` can build and no `.cfe` at all, while `yaxunit` gets the `tests` source-set. Omit `sources` for the ready artifact: `build/tools/client_mcp.cfe`, which is what `tools.client_mcp.extension.artifact.path` and the `build` preflight expect, or `build/tools/YAxUnit-<version>.cfe`.
