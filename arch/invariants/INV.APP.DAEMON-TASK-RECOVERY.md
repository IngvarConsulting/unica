---
id: INV.APP.DAEMON-TASK-RECOVERY
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check:
  - crates/unica-coder/src/infrastructure/task_store.rs::v1_nonresumable_working_record_recovers_as_interrupted_failure
  - crates/unica-coder/src/infrastructure/task_store.rs::v1_resumable_working_without_a_registered_owner_recovers_as_unsupported
  - crates/unica-coder/src/infrastructure/task_store.rs::v1_terminal_record_migrates_to_v2_without_changing_its_domain_result
  - crates/unica-coder/src/infrastructure/task_store.rs::v1_failed_and_cancelled_terminal_records_migrate_deterministically
  - crates/unica-coder/src/infrastructure/task_store.rs::v2_working_without_a_live_owner_recovers_as_interrupted
  - crates/unica-coder/src/infrastructure/task_store.rs::v2_working_resume_descriptor_without_registered_owner_is_unsupported
  - crates/unica-coder/src/infrastructure/task_store.rs::v2_failed_record_persists_only_a_closed_failure_reason
  - crates/unica-coder/src/infrastructure/task_store.rs::unknown_record_schema_fails_closed_instead_of_reinterpreting_bytes
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
