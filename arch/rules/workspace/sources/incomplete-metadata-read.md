---
id: INV.SOURCE.INCOMPLETE-METADATA-READ
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/service_member_facts.rs::service_member_damage_limits_only_the_named_fact_and_keeps_owner_diagnostics
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_attribute_tooltip.rs::catalog_attribute_tooltip_damage_is_local_in_owner_row_and_detail
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_attribute_tooltip.rs::catalog_attribute_tooltip_empty_set_reads_as_absence_but_an_empty_translation_survives
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_attribute_properties.rs::catalog_attribute_profile_damage_limits_only_the_named_fact_in_owner_row_and_detail
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_attribute_properties.rs::catalog_attribute_profile_absence_is_not_filled_from_parent_sibling_or_template
  - crates/unica-metadata/src/member_read_profile/tests.rs::member_profile_owns_only_exact_context_property_diagnostics
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_tabular_properties.rs::catalog_tabular_profile_damage_is_local_to_the_observed_fact
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_tabular_properties.rs::catalog_tabular_profile_missing_facts_are_not_filled_from_sibling_or_template
  - crates/unica-metadata-designer/src/form_metadata.rs::form_use_purposes_preserves_absence_empty_and_malformed_observations
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::specialized_relation_limits_distinguish_absence_empty_and_damage
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::missing_relation_preserves_known_targets_and_reports_incompleteness
  - crates/unica-coder/src/infrastructure/native_operations/meta/info_tests.rs::missing_write_mode_does_not_claim_complete_registrar_evidence
  - crates/unica-metadata-designer/tests/platform/info_projection.rs::property_observation_distinguishes_missing_false_empty_nil_and_damage
  - crates/unica-coder/src/infrastructure/native_operations/meta/info_tests.rs::platform_incomplete_borrowed_metadata_keeps_local_observations
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::missing_or_damaged_code_type_limits_only_dependent_predefined_read
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::platform_borrowed_common_module_preserves_unknown_contexts_and_independent_facts
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::unknown_facts_of_borrowed_objects_are_named_as_node_limits
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/inherited_content_tests.rs::predefined_writes_refuse_an_owner_without_code_type
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::predefined_writer_refuses_an_owner_without_code_type_like_the_reader
---

# Неполное описание сохраняет независимые локальные сведения

Чтение метаданных различает присутствующее корректное значение, отсутствие
свойства и ошибку его представления. Явные `false` и допустимые пустые строки
сохраняются; `nil`, пустое значение и многоязычная строка без переводов в
ответе `view` свойства не создают, как описано в
[metadata-node-properties.md](metadata-node-properties.md). Допустимость
отсутствия определяется видом объекта и свойством в профиле чтения,
независимо от разрешений на изменение.

Отсутствующее значение не подставляется из шаблона создания или родителя.
Если для зависимого вывода недостаточно сведений, ответ сообщает ограничение
и сохраняет доступные независимые факты. Повреждённое присутствующее значение
сохраняет диагностику. У отсутствующего применимого списка связей неизвестна
полнота; явно пустой контейнер остаётся известным пустым списком. Ограничение
видно в `limits` как у объекта, так и в ветви `Relation` вместе с доступными
ссылками. Так же предел называет регистр сведений без `WriteMode`: список его
регистраторов неизвестен. Неизвестные контексты общего модуля равны `null`,
а узел модуля и его ветви называют причину пределом.

Без `CodeType` владельца вид кодов предопределённых элементов неизвестен:
чтение ограничивает ветвь предопределённых, а запись предопределённых
отказывает тем же текстом, не проверяя коды по придуманному виду.

Ошибка одного из наблюдаемых свойств табличной части `LineNumberLength`
или `Use` оставляет саму часть, остальные доказанные свойства и колонки
доступными. Ограничение видно у объекта, в строке коллекции и в детали
табличной части. Допустимое отсутствие этих свойств не создаёт значения
по умолчанию или ошибки.

Для `Indexing`, `FullTextSearch` и `DataHistory` её прямого прикладного
реквизита действует та же независимость фактов: повреждение одного свойства
сохраняет реквизит, исправные свойства и тип. Ограничение видно у владельца,
в строке коллекции и в детали реквизита. Допустимое отсутствие не создаёт
значение по умолчанию; пустой тег этих enum-свойств считается повреждением.

Локализованный `ToolTip` прямого прикладного реквизита того же профиля
различает подсказку с переводами, её отсутствие и повреждение; пустой
контейнер читается как отсутствие. Голый текст вместо
локализованной структуры, повтор языка и некорректные узлы не превращаются
в пустую подсказку. Повреждение ограничивает этот факт у владельца, в строке
коллекции и в детали, сохраняя остальные сведения реквизита.

Проверки идентичности и структуры объекта остаются обязательными. Чтение
не изменяет исходники.

Правила общего модуля уточнены в [module-common-properties.md](module-common-properties.md).
Разрешение родителя следует [borrowed-object-parent.md](borrowed-object-parent.md).
Отсутствие у заимствованного объекта не свойства описания, а сопутствующего
файла содержимого описано в
[borrowed-object-inherited-content.md](borrowed-object-inherited-content.md).
Эти гарантии не определяют итоговые свойства объединённой конфигурации.
