---
id: INV.SOURCE.XDTO-LOCAL-CHANGE
check:
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_package_mapping_is_logical_and_single_resource
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_v12_parity_preserves_exact_bytes_errors_and_noop
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_returns_one_local_patch_and_preserves_outside_bytes
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_preserves_crlf_and_uses_local_eol_in_a_mixed_document
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_fails_closed_when_mixed_eol_has_no_local_context
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_removal_matrix_preserves_exact_outer_slices
---

# Изменение схемы XDTO сохраняет остальной текст пакета

Добавление и удаление типов и свойств меняет только выбранный
`XDTOPackages/<Имя>/Ext/Package.bin`. Дескриптор объекта метаданных остаётся
отдельным ресурсом.

Пакет изменяется локально: байты за пределами правки, начальный BOM
и переводы строк сохраняются. Для вставки используется перевод строки
рядом с целью; при смешанном стиле без такого образца писатель отказывает.
Проверки проходят планировщик операций и писатель пакета.
