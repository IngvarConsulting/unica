# Troubleshooting

- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

Classify failures before retrying:

- License text means hard stop. Do not repair licensing automatically.
- Classify only authentication evidence supplied by the user or another verified boundary; do not initiate a runtime probe while applied execution is blocked. Without credentials, ask the user instead of cycling accounts.
- Missing platform, runner, tool, VA, or MCP extension is an environment/setup issue; report the exact missing component.
- Stale generated state after branch switch or rebase should preview `build` with `fullRebuild=true`; applied build is currently fail-closed before spawn.
- Missing project config, artifact, or managed tool payload may be previewed with `config-init`, `make`, or `tools-download`, but their applied persistent writes are currently fail-closed before spawn; do not bypass them with a direct runner invocation.
- Only a durable build carries the one full retry after a failed partial load. The synchronous entry point owns exactly one process and never repeats it, so its terminal result is not the fallback declining.
- Empty `unica.runtime.job.logs` during a default durable `build` is expected, not a hang: that build runs with `--json-message`, so the runner emits one structured envelope at exit and streams no progress. Judge liveness from `phase` and the heartbeat in `unica.runtime.job.status`; use `fullRebuild=true` when the streamed text output matters more than the automatic retry.
- That retry is automatic only when external exit code `4` accompanies a valid structured failure for the completed partial load step. Treat it as stage evidence, not a diagnosis of vendor support or any other cause. When the pinned exit code arrives and the retry still does not happen, the job warning names the check that refused the receipt.
- Do not retry arbitrary or malformed errors, process spawn failures, cancellation, a process timeout observed by Unica, truncated output, or failures from another build step. Explicit `fullRebuild=true` and a failed full fallback also do not start another attempt.
- The pinned receipt has no deferred internal timeout metadata. If a critical runner step crosses its own deadline and later returns the exact completed partial failure, the temporary Unica layer cannot distinguish that case and may still start the one full retry.
- The support-independent fallback is temporary; the runtime/runner redesign for v14 is a separate change.
- Unexpected source changes after dump should be reviewed as a Git diff before continuing.

Do not bypass typed MCP arguments with raw shell flags. If the needed v8-runner flag is missing from `unica.runtime.execute`, treat that as a Unica MCP contract gap.
