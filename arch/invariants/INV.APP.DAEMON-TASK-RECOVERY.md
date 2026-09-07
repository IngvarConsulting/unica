---
id: INV.APP.DAEMON-TASK-RECOVERY
status: active
governs: product
decision: DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER
check:
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::recovery_terminalizes_queued_without_starting_domain_work
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::recovered_begun_task_is_created_working_and_keeps_cancel_intent
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5.rs::startup_terminalizes_pre_task_receipts_without_replaying_domain_work
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5.rs::startup_materializes_handoff_without_replaying_begun_work
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::restart_begun_without_committed_handoff_is_direct_outcome_uncertain
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::working_readback_before_receipt_begun_recovers_interrupted_without_callback
scope: [app, cache]
---

# Restart recovery не изображает resume без зарегистрированного owner

Store строго читает schema v1 и v2. На этом срезе resume owners отсутствуют:
любой v1/v2 `Working` после restart становится durable failed record с закрытой
причиной `Interrupted` или `ResumeUnsupported`, но не запускается повторно и не
остаётся `Working`. V1 terminal мигрирует в v2 без изменения DomainResult;
неизвестная schema и неизвестные поля отклоняются. Enumeration и размер records
имеют жёсткие границы; превышение отклоняется типизированно до неограниченного
чтения. Это же правило закрывает на следующем open запись, оставшуюся `Working`
после смерти `RestartRequested` процесса из-за неподтверждаемой durable
publication.
