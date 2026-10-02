---
id: INV.SOURCE.ROLE-RIGHT-ADDRESSES
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::role_merges_access_by_canonical_object_and_keeps_rls_under_that_right
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::ambiguous_short_role_alias_is_rejected_and_canonical_aliases_work
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_role_canonical_encoding_cannot_collapse_distinct_kind_name_pairs
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_role_rejects_non_platform_metadata_node_kinds
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::configuration_level_rights_are_readable_role_objects
---

# Права роли адресуются по виду и имени объекта

Разрешённые и запрещённые права на один объект объединяются в узле `Right`.
Его адрес содержит вид и имя, например `Catalog_Orders`; ограничения RLS
находятся только под этим узлом и не дублируются.

Короткое имя допустимо, пока оно однозначно. Если есть и справочник,
и документ `Orders`, короткий адрес отклоняется с `bad_value` и вариантами
`Catalog_Orders` и `Document_Orders`.

Допускаются виды метаданных из реестра платформы и `Configuration` для
прав на конфигурацию в целом. Неизвестный вид даёт `provider_unavailable`,
а не адрес, который может совпасть с адресом другого объекта.
