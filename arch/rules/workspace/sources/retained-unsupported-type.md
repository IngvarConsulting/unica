---
id: INV.SOURCE.RETAINED-UNEDITABLE-TYPE
check:
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests/retained_readonly_type.rs::canonical_independent_attribute_edits_preserve_read_only_type_bytes
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests/retained_readonly_type.rs::canonical_fill_value_refuses_read_only_type_without_source_changes
---

# Правка другого свойства сохраняет неизменяемый тип

Если тип реквизита доступен только для чтения, изменение другого независимого
свойства сохраняет его исходный XML-фрагмент без преобразования в модель
записи. Изменение самого типа или зависящего от него значения допускается
только при поддержанной возможности записи.
