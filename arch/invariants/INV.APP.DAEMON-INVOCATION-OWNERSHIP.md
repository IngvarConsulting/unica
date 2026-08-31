---
id: INV.APP.DAEMON-INVOCATION-OWNERSHIP
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/infrastructure/daemon/mod.rs::daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it
scope: [app]
---

# Одна canonical Invocation имеет один daemon execution

Явно выбранный canonical V13 вызов исполняется daemon ровно один раз. Get, wait,
повторная cancel и повторное чтение состояния не принимают domain callback и не
могут запустить вызов снова. Frontend transport failure не разрешает fallback
на локальный обработчик.
