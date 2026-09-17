---
id: INV.APP.DAEMON-INVOCATION-OWNERSHIP
status: active
governs: product
decision: DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::exact_duplicate_preserves_cutoff_without_second_domain_callback
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::crash_after_begun_returns_outcome_uncertain_without_replay
scope: [app]
---

# Одна canonical Invocation имеет один daemon execution

Явно выбранный canonical V13 вызов исполняется daemon ровно один раз. Get, wait,
повторная cancel и повторное чтение состояния не принимают domain callback и не
могут запустить вызов снова. Frontend transport failure не разрешает fallback
на локальный обработчик.
