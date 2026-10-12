---
id: INV.SOURCE.INCOMPLETE-METADATA-READ
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::specialized_relation_limits_distinguish_absence_empty_and_damage
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::missing_relation_preserves_known_targets_and_reports_incompleteness
  - crates/unica-coder/src/infrastructure/native_operations/meta/info_tests.rs::missing_write_mode_does_not_claim_complete_registrar_evidence
  - crates/unica-coder/src/infrastructure/native_operations/meta/info_projection_tests.rs::property_observation_distinguishes_missing_false_empty_nil_and_damage
  - crates/unica-coder/src/infrastructure/native_operations/meta/info_tests.rs::platform_incomplete_borrowed_metadata_keeps_local_observations
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::missing_or_damaged_code_type_limits_only_dependent_predefined_read
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::platform_borrowed_common_module_preserves_unknown_contexts_and_independent_facts
---

# Неполное описание сохраняет независимые локальные сведения

Чтение метаданных различает присутствующее корректное значение, отсутствие
свойства и ошибку его представления. Явные `false`, допустимые пустые значения
и `nil` сохраняются. Допустимость отсутствия определяется видом объекта и
свойством в профиле чтения, независимо от разрешений на изменение.

Отсутствующее значение не подставляется из шаблона создания или родителя.
Если для зависимого вывода недостаточно сведений, ответ сообщает ограничение
и сохраняет доступные независимые факты. Повреждённое присутствующее значение
сохраняет диагностику. У отсутствующего применимого списка связей неизвестна
полнота; явно пустой контейнер остаётся известным пустым списком. Ограничение
видно как у объекта, так и в ветви `Relation` вместе с доступными ссылками. Проверки идентичности и структуры объекта остаются
обязательными. Чтение не изменяет исходники.

Правила общего модуля уточнены в [module-common-properties.md](module-common-properties.md).
Разрешение родителя следует [borrowed-object-parent.md](borrowed-object-parent.md).
Отсутствие у заимствованного объекта не свойства описания, а сопутствующего
файла содержимого описано в
[borrowed-object-inherited-content.md](borrowed-object-inherited-content.md).
Эти гарантии не определяют итоговые свойства объединённой конфигурации.
