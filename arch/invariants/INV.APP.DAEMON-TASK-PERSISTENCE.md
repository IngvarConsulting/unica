---
id: INV.APP.DAEMON-TASK-PERSISTENCE
status: active
governs: product
decision: DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER
check:
  - crates/unica-coder/src/application/invocation_store_v5.rs::all_five_task_statuses_round_trip_with_the_exact_selected_fields
  - crates/unica-coder/src/application/invocation_store_v5.rs::record_rejects_wrong_schema_unknown_duplicate_and_every_missing_root_field
  - crates/unica-coder/src/application/invocation_store_v5.rs::v5_safe_failure_reason_is_closed_and_converts_every_legacy_reason
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
