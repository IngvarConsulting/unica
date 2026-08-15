# Troubleshooting

- По INV-MCP-RUNTIME-RECEIPT текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.

Classify failures before retrying:

- License text means hard stop. Do not repair licensing automatically.
- Classify only authentication evidence supplied by the user or another verified boundary; do not initiate a runtime probe while applied execution is blocked. Without credentials, ask the user instead of cycling accounts.
- Missing platform, runner, tool, VA, or MCP extension is an environment/setup issue; report the exact missing component.
- Stale generated state after branch switch or rebase should preview `build` with `fullRebuild=true`; applied build is currently fail-closed before spawn.
- Missing project config, artifact, or managed tool payload may be previewed with `config-init`, `make`, or `tools-download`, but their applied persistent writes are currently fail-closed before spawn; do not bypass them with a direct runner invocation.
- Unexpected source changes after dump should be reviewed as a Git diff before continuing.

Do not bypass typed MCP arguments with raw shell flags. If the needed v8-runner flag is missing from `unica.runtime.execute`, treat that as a Unica MCP contract gap.
