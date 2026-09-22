---
id: INV.SOURCE.XDTO-PROPERTY-PATH
check:
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_rejects_every_invalid_property_path_shape
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_addresses_a_dotted_property_identity_with_an_escape
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_separates_only_unescaped_property_path_dots
---

# Точка в имени свойства XDTO отличается от перехода к вложенному свойству

В `propertyPath` обычная точка разделяет вложенные свойства, а `\.` означает
буквальную точку имени. Например, `A\.B.Child` выбирает свойство `Child`
внутри `A.B`. Переход проходит через вложенное определение типа `typeDef`.
Пустые сегменты и иное экранирование отклоняются.
Проверки проходят внутренний писатель; тот же аргумент принимает текущий
планировщик XDTO.
