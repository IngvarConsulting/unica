---
id: INV.APP.DAEMON-TASK-RECOVERY
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/infrastructure/task_store.rs::schema_v2_recovery_never_false_resumes_and_persists_only_closed_failure_reasons
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
