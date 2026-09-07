---
id: INV.APP.EXACT-LONG-WORK-OWNERSHIP
status: active
governs: product
decision: DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::production_known_long_task_executes_after_the_initial_working_projection
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::known_long_requires_begun_bound_handoff_intent
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::known_long_after_prepare_never_becomes_unbound_promise
scope: [app, cache]
---

# Долгая readiness-работа разделяется без потери actor и lease authority

Index разделяется только для одного actor, source set и полной trusted revision;
новая revision, другой worktree или заменённый root не получают старый staged
result. ProviderHost может быть общим для двух worktree только по совпадающим
engine, target и capabilities, тогда как их actor-bound чтения, результаты и
cache state остаются различны. Runtime разделяется только по exact resource и
существующему active job lease; ожидание начинается после durable Task handoff.
