---
id: INV.APP.DAEMON-TASK-PERSISTENCE
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/infrastructure/daemon/mod.rs::durable_handoff_persists_only_closed_hashes_not_arguments_paths_or_failure_text
scope: [app, cache]
---

# Durable handoff сохраняет только закрытый Task allowlist

Task record содержит закрытый ToolIdentity, нормализованный digest arguments,
daemon-derived workspace identity, закрытые status и допустимый DomainResult.
Raw arguments, caller paths, stdout, stderr и свободный runtime failure text в
record не попадают. Ошибка Task строится из закрытого status при чтении.
