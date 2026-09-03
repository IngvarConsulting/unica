---
id: INV.APP.DAEMON-TASK-PERSISTENCE
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/infrastructure/daemon/mod.rs::durable_handoff_persists_only_closed_hashes_not_arguments_paths_or_failure_text
scope: [app, cache]
---

# Durable handoff сохраняет только закрытый Task allowlist

Task record schema v2 содержит закрытый ToolIdentity, нормализованный digest
arguments, actor-derived workspace identity, закрытые status,
`SafeFailureReason` и допустимый DomainResult. Raw arguments, caller/runtime
text, stdout, stderr и свободный failure text в record не попадают. Ошибка Task
строится только из закрытой причины при чтении.
Canonical DomainResult ограничен 8 MiB и одинаково проверяется для direct и
Task. Persistent envelope имеет отдельный запас 64 KiB и не расширяет result.
Превышение сохраняется только закрытой причиной `ResultTooLarge`, без bytes
результата.
