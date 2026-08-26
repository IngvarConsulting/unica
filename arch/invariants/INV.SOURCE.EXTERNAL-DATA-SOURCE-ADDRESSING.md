---
id: INV.SOURCE.EXTERNAL-DATA-SOURCE-ADDRESSING
status: active
governs: product
decision: DEC.2026-08-22.EXTERNAL-DATA-SOURCE-METADATA
check: crates/unica-coder/src/domain/source_target.rs::external_data_source_addresses_reach_tables_cubes_and_dimension_tables
scope: [source]
---

# Адрес внешнего источника имеет закрытую глубину

После `ExternalDataSource.<Имя>` допустимы `Table.<Имя>` или `Cube.<Имя>`, а
после куба — один `DimensionTable.<Имя>`; роль модуля завершает только полный
адрес поддержанного объекта.
