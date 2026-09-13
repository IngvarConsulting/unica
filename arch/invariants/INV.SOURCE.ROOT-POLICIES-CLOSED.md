---
id: INV.SOURCE.ROOT-POLICIES-CLOSED
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/format_guard.rs::unknown_version_bearing_roots_are_rejected_by_the_closed_policy_catalog
scope: [source]
---

# Неизвестный QName не получает политику корня

Версионированный QName вне закрытого каталога platform XML фактически
отклоняется гейтом записи как `formatVersionInvalid`, а не получает неявную
политику публикации или роль владельца.
