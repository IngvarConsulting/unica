---
id: INV.APP.EXTERNAL-DATA-SOURCE-READ-MODEL
status: active
governs: product
decision: DEC.2026-08-22.EXTERNAL-DATA-SOURCE-METADATA
check: crates/unica-coder/src/infrastructure/native_operations/meta/info_projection_tests.rs::external_data_source_details_observe_registered_tables_and_cubes
scope: [app, source]
---

# Read model внешнего источника различает пустоту и недоступность

`details.tables` и `details.cubes` содержат логические адреса доказанных детей:
доказанное отсутствие равно `[]`, недоступный или повреждённый состав равен
`null` с диагностикой.
