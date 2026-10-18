---
id: INV.SOURCE.METADATA-NODE-PROPERTIES
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_tabular_properties.rs::catalog_tabular_profile_reaches_public_collection_and_detail_without_writes
  - crates/unica-coder/src/infrastructure/v13_read_projection/member_properties/tests.rs::catalog_tabular_provider_profile_does_not_publish_unknown_names_or_raw_item_keys
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::form_projection_uses_a_positive_nested_scalar_allowlist
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::role_right_projection_never_serializes_an_unbounded_rights_array_into_props
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::metadata_node_props_carry_the_observed_object_properties
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::metadata_node_props_lay_out_the_per_kind_facts_by_role
---

# Свойства объекта доступны в его логическом узле

`props` узла метаданных содержит наблюдаемые свойства под ключами из
закрытого словаря профиля вида. Например, справочник публикует
`Hierarchical` и `CodeLength` под этими именами.

Составной факт с отдельными ролями раскладывается на плоские поля:
обработчик задания — `handlerModule` и `handlerMethod`, расписание регистра
расчёта — `scheduleRegister`, `scheduleValueField` и `scheduleDateField`.
Остальные составные значения, например тип константы, приходят компактной
строкой, если помещаются в ограничение размера `props`. Вложенные объекты
и массивы в `props` не публикуются.

Для прямой табличной части справочника обычной конфигурации в XML 2.20
коллекция и деталь публикуют наблюдённые `LineNumberLength` числом и `Use`
строкой из закрытого словаря профиля чтения. Этот профиль не определяет
разрешения на изменение свойств.

Для типизированных дочерних узлов действует закрытый набор допустимых
свойств: неизвестное поле поставщика не становится свойством элемента формы.
Список прав не сериализуется целиком в скалярное свойство узла права роли;
компактное текстовое свойство не превышает 2048 байт.
Эти ограничения не запрещают специально предусмотренные составные факты
корня `Configuration`, например его начальную страницу и командный интерфейс.
