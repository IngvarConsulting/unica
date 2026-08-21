---
id: INV.SOURCE.PORTABLE-GIT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/tests/platform/project_health.rs::project_health_full_portable_repository_is_ready
scope: [source]
---

# Переносимость Git доказывается содержимым репозитория

`repositoryReady` вычисляется отдельно от `ready` и требует отслеживаемых
правил исключений, ролевой классификации атрибутов и окончаний строк выгрузки
платформы и безопасной классификации подготовленного `ConfigDumpInfo.xml`.
Локальные правила не считаются переносимыми, а отдельное хранилище больших
файлов остаётся необязательной подсказкой и не меняет ни один флаг.
