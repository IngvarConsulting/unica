---
id: INV.SOURCE.MODULE-COMMON-PROPERTIES
check:
  - crates/unica-bsl/src/projection.rs::common_module_flags_serialize_exactly_once_and_never_become_contexts
  - crates/unica-bindings-integration-tests/tests/projection.rs::common_module_requires_all_normalized_flags
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_keeps_missing_privileged_unknown
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_view_serializes_unknown_privileged_without_losing_contexts
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_reads_platform_absent_properties_as_unknown
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_view_keeps_absent_properties_and_contexts_unknown
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_without_any_single_environment_flag_has_unknown_contexts
  - crates/unica-coder/src/infrastructure/daemon/server.rs::borrowing_view_bounds_override_props_for_metadata_and_specialized_readers
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::platform_borrowed_common_module_preserves_unknown_contexts_and_independent_facts
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::common_module_absence_policy_keeps_ordinary_properties_required
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::common_module_present_damage_is_not_missing
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_common_module_missing_reuse_preserves_methods_and_source
  - crates/unica-bindings-integration-tests/tests/projection.rs::unknown_execution_flags_do_not_become_empty_or_universal_contexts
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::unknown_facts_of_borrowed_objects_are_named_as_node_limits
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
родителя. Явно указанное некорректное значение остаётся ошибкой источника,
как и повреждённая идентичность заимствования: дубликат, чужое пространство
имён, вложенная разметка или неверное значение `ObjectBelonging` и
`ExtendedConfigurationObject` не открывают допуска к пропускам. Платформенная
форма подтверждена и вторым опытом 8.3.27.2130 в
`tests/fixtures/platform_8_3_27/incomplete_borrowed/`.
Построитель требует сам объект свойств целиком.

Если у заимствованного модуля не записан хотя бы один из четырёх флагов среды,
контексты компиляции его методов и условных областей неизвестны и равны
`null`, а ответ о модуле называет это пределом в `limits`.

`serverCall`, `privileged` и `returnValuesReuse` не являются отдельными
контекстами компиляции. Проверки охватывают наличие данных на входе
построителя и форму сериализации сводки; расчёт контекстов по разным
сочетаниям флагов требует проверки из `gap`.
