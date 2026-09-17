---
id: INV.SOURCE.METADATA-NODE-DETAIL-ROLES
status: active
governs: product
decision: DEC.2026-09-10.PER-KIND-FACTS-LAY-OUT-BY-ROLE
check: crates/unica-coder/src/infrastructure/v13_read/tests.rs::metadata_node_props_lay_out_the_per_kind_facts_by_role
scope: [product, source]
---

# Составной факт вида отвечает ключом на роль

Поле составного факта вида метаданных приходит в `props` отдельным ключом,
названным по роли поля, а не по пути внутри составного значения. Объектом в
`props` составной факт не приходит никогда.
