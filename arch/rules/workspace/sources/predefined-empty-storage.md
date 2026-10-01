---
id: INV.SOURCE.PREDEFINED-EMPTY-STORAGE
check:
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::removing_the_last_item_deletes_the_file_like_the_platform_export
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::unchanged_or_absent_code_type_companion_keeps_precise_preimage_guards
---

# Удаление последнего предопределённого элемента удаляет его файл

После удаления последнего предопределённого элемента Unica удаляет
`Ext/Predefined.xml`, а не записывает пустой документ. Изменение типа кода
(`CodeType`) у объекта без предопределённых элементов также не создаёт этот файл.

Проверки подтверждают план изменения файлов. Правило относится только
к предопределённым данным; состав других файлов задаётся их форматом.
