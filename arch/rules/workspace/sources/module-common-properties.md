---
id: INV.SOURCE.MODULE-COMMON-PROPERTIES
check:
  - crates/unica-coder/src/domain/module_projection.rs::common_module_flags_serialize_exactly_once_and_never_become_contexts
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::common_module_requires_all_normalized_flags
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_keeps_missing_privileged_unknown
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_view_serializes_unknown_privileged_without_losing_contexts
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_reads_platform_absent_properties_as_unknown
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_view_keeps_absent_properties_and_contexts_unknown
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_without_any_single_environment_flag_has_unknown_contexts
  - crates/unica-coder/src/infrastructure/daemon/server.rs::borrowing_view_bounds_override_props_for_metadata_and_specialized_readers
gap: https://github.com/IngvarConsulting/unica/issues/973
---

# Свойства общего модуля не подменяют контексты компиляции

Описание общего модуля содержит по одному ключу `global`,
`clientManagedApplication`, `server`, `externalConnection`,
`clientOrdinaryApplication`, `serverCall`, `privileged` и `returnValuesReuse`.
Читатель требует значения всех свойств у обычного модуля и у собственного
модуля расширения. Платформа 8.3.27 принимает и выгружает заимствованный
расширением `CommonModule` без любого из этих свойств и не переносит в него
значения родителя. Незаписанное свойство заимствованного модуля равно
`null`: отсутствие сведения не превращается в `false`, `DontUse` или значение
родителя. Явно указанное некорректное значение остаётся ошибкой источника.
Построитель требует сам объект свойств целиком.

Если у заимствованного модуля не записан хотя бы один из четырёх флагов среды,
контексты компиляции его методов и условных областей неизвестны и равны
`null`; фильтр `context` по такому модулю отказывает `unsupported_filter`,
а не включает или исключает методы.

`serverCall`, `privileged` и `returnValuesReuse` не являются отдельными
контекстами компиляции. Проверки охватывают наличие данных на входе
построителя и форму сериализации сводки; расчёт контекстов по разным
сочетаниям флагов требует проверки из `gap`.
