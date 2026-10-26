---
id: INV.SOURCE.LOCALIZED-STRING-NSTR
check:
  - crates/unica-metadata/src/properties.rs::nstr_round_trips_quotes_and_rejects_text_without_a_language
  - crates/unica-metadata/src/property_input.rs::descriptor_localized_properties_take_the_whole_nstr_value
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::synonym_takes_the_whole_value_in_nstr_form
  - crates/unica-coder/src/infrastructure/v13_read_projection/member_properties/tests/catalog_attribute.rs::catalog_attribute_provider_tooltip_publishes_localized_values_in_row_and_detail
  - crates/unica-coder/src/infrastructure/v13_read_projection/member_properties/tests/catalog_attribute.rs::catalog_attribute_provider_tooltip_rejects_wrong_shapes_and_duplicate_languages_locally
gap:
  - https://github.com/IngvarConsulting/unica/issues/2163
  - https://github.com/IngvarConsulting/unica/issues/2194
---

# Многоязычная строка передаётся в форме НСтр

Значение локализуемого свойства — синонима, представлений, пояснения,
подсказки, сведений о конфигурации и других — читается и записывается одной
строкой в форме НСтр 1С: пары `код = 'текст'` через точку с запятой, например
`ru = 'Валюты'; en = 'Currencies'`. Одинарная кавычка внутри текста
удваивается. Форма та же при одном языке и одинакова в `view`, по адресу
`<узел>.Props.<имя>` и в записи.

Чтение перечисляет все переводы в порядке источника. Свойство без переводов
не создаётся.

Запись строкой НСтр задаёт полное значение: после неё у свойства ровно
перечисленные языки в указанном порядке, языков, которых в строке нет, у
свойства не остаётся. Пустая строка очищает свойство. Строка, которая не
разбирается как НСтр, в том числе текст без кода языка, и строка с повторным
кодом языка отклоняются `bad_value` с указанием формы.
