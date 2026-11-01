---
id: INV.SOURCE.METADATA-NODE-PROPERTIES
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/commonpicture_descriptor.rs::commonpicture_descriptor_preserves_explicit_xml_facts_without_payload
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_attribute_tooltip.rs::catalog_attribute_tooltip_completes_six_facts_in_public_collection_and_detail_without_writes
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_attribute_tooltip.rs::catalog_attribute_tooltip_compact_limit_preserves_independent_facts
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_attribute_properties.rs::catalog_attribute_profile_reaches_public_collection_and_detail_without_writes
  - crates/unica-coder/src/infrastructure/v13_read_projection/member_properties/tests/catalog_attribute.rs::catalog_attribute_provider_keeps_contexts_closed_and_ignores_raw_item_keys
  - crates/unica-coder/src/infrastructure/v13_read/tests/catalog_tabular_properties.rs::catalog_tabular_profile_reaches_public_collection_and_detail_without_writes
  - crates/unica-coder/src/infrastructure/v13_read_projection/member_properties/tests.rs::catalog_tabular_provider_profile_does_not_publish_unknown_names_or_raw_item_keys
  - crates/unica-coder/src/infrastructure/v13_read/tests/form.rs::form_projection_uses_a_positive_nested_scalar_allowlist
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::role_right_projection_never_serializes_an_unbounded_rights_array_into_props
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::metadata_node_props_carry_the_observed_object_properties
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::metadata_node_props_lay_out_the_per_kind_facts_by_role
  - crates/unica-coder/src/infrastructure/v13_read_projection.rs::an_oversize_value_names_its_whole_address_and_an_empty_one_does_not
---

# Свойства объекта доступны в его логическом узле

`props` узла метаданных содержит наблюдаемые свойства под ключами из
закрытого словаря профиля вида. Например, справочник публикует
`Hierarchical` и `CodeLength` под этими именами.

Составной факт с отдельными ролями раскладывается на плоские поля:
обработчик задания — `handlerModule` и `handlerMethod`, расписание регистра
расчёта — `scheduleRegister`, `scheduleValueField` и `scheduleDateField`.
Остальные составные значения, например тип константы, приходят вложенным
JSON, а не строкой с JSON внутри. У наблюдаемого свойства описания обёртка
снимается: текст, флаг и число приходят простым значением, а `nil`, пустое
значение и многоязычная строка без переводов свойства не создают. Значение
типизированного поля, например `type` или `fillValue`, сохраняет форму,
которую принимает `apply`. Составное значение,
которое в сериализованном виде больше 2048 байт, не публикуется, а ответ
называет превышенный предел в `limits` и адрес `<узел>.Props.<имя>`, по
которому `view` отдаёт свойство целиком в той же форме, что и в `props`.

Для прямой табличной части справочника обычной конфигурации в XML 2.20
коллекция и деталь публикуют наблюдённые `LineNumberLength` числом и `Use`
строкой из закрытого словаря профиля чтения. Этот профиль не определяет
разрешения на изменение свойств.

У прямого прикладного реквизита такой табличной части коллекция и деталь
публикуют наблюдённые `Indexing`, `FullTextSearch` и `DataHistory` строками
из закрытых словарей. Контекст реквизита отделён от контекста самой части:
одноимённые поля других узлов не получают этот профиль. Наблюдение сохраняет
действующие разрешения на запись.

У такого реквизита `ToolTip` сохраняет языки, тексты и порядок локализованных
значений. В коллекции и детали он представлен массивом переводов
`{language, content}` в пределах общего ограничения размера свойства.
Пустой набор переводов, как и отсутствие подсказки, свойства не создаёт.
Этот факт чтения не расширяет разрешения на изменение.

Для типизированных дочерних узлов действует закрытый набор допустимых
свойств: неизвестное поле поставщика не становится свойством элемента формы.
Список прав не публикуется целиком в свойстве узла права роли; свойство
узла, текстовое или составное, не превышает 2048 байт.
Эти ограничения не запрещают специально предусмотренные составные факты
корня `Configuration`, например его начальную страницу и командный интерфейс,
и объект `properties` корня формы.
