---
id: INV.SOURCE.ROOT-SEPARATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/project_health/layout.rs::root_identity_equal_to_workspace_is_one_primary_fact
scope: [source]
---

# Корень исходников отделён от рабочего пространства

Полная инспекция считает каждый уникально адресуемый корень набора исходников
строгим потомком корня рабочего пространства. Равенство после нормализации,
включая `path: .`, `./` и `src/..`, даёт одну первичную ошибку
`source_set.root_is_workspace` и не порождает производные ошибки о служебных
путях внутри того же корня.
