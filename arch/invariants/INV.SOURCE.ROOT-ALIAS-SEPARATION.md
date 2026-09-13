---
id: INV.SOURCE.ROOT-ALIAS-SEPARATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/project_health/layout.rs::linked_alias_to_workspace_reports_the_primary_identity_cause_with_evidence
scope: [source]
---

# Ссылочный псевдоним не скрывает корень рабочего пространства

Физически совпадающий с рабочим пространством ссылочный корень даёт единственную
первичную ошибку `source_set.root_is_workspace` с доказательством связанного
маршрута и не запускает производные проверки этого корня.
