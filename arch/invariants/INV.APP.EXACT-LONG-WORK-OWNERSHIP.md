---
id: INV.APP.EXACT-LONG-WORK-OWNERSHIP
status: active
governs: product
decision: DEC.2026-08-24.LONG-WORK-OWNERSHIP-SLICE
check: crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_exact_long_work_ownership_contract
scope: [app, cache]
---

# Долгая readiness-работа разделяется без потери actor и lease authority

Index разделяется только для одного actor, source set и полной trusted revision;
новая revision, другой worktree или заменённый root не получают старый staged
result. ProviderHost может быть общим для двух worktree только по совпадающим
engine, target и capabilities, тогда как их actor-bound чтения, результаты и
cache state остаются различны. Runtime разделяется только по exact resource и
существующему active job lease; ожидание начинается после durable Task handoff.
