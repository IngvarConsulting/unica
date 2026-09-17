---
id: INV.SOURCE.PORTABLE-GIT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/tests/platform/project_health.rs::portable_git_readiness_contract_is_a_closed_positive_and_negative_matrix
scope: [source]
---

# Переносимость Git доказывается содержимым репозитория

`repositoryReady` вычисляется отдельно от `ready` и требует отслеживаемых
правил исключений, ролевой классификации атрибутов и окончаний строк выгрузки
платформы и безопасной классификации подготовленного `ConfigDumpInfo.xml`.
Локальные правила не считаются переносимыми; отрицательная матрица отдельно
закрывает некорректный `ConfigDumpInfo.xml`, отсутствующую политику текста,
локальные атрибуты и неполный осмотр конкретного набора исходников.
