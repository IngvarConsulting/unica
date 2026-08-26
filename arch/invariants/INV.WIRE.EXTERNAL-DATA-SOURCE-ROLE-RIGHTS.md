---
id: INV.WIRE.EXTERNAL-DATA-SOURCE-ROLE-RIGHTS
status: active
governs: product
decision: DEC.2026-08-22.EXTERNAL-DATA-SOURCE-METADATA
check: crates/unica-coder/src/infrastructure/native_operations/role.rs::external_data_source_rights_round_trip_through_public_edit_path
scope: [wire, source]
---

# Role edit принимает доказанные права внешнего источника

Типизированный публичный путь сохраняет только разрешённые права корневого
`ExternalDataSource`, его таблицы и поля таблицы и отклоняет неподтверждённые
сочетания до публикации.
