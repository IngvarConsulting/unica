---
id: INV.SOURCE.XDTO-LOCAL-CHANGE
check:
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_package_mapping_is_logical_and_single_resource
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_v12_parity_preserves_exact_bytes_errors_and_noop
  - crates/unica-xdto-designer/src/writer.rs::xdto_writer_returns_one_local_patch_and_preserves_outside_bytes
  - crates/unica-xdto-designer/src/writer.rs::xdto_writer_preserves_crlf_and_uses_local_eol_in_a_mixed_document
  - crates/unica-xdto-designer/src/writer.rs::xdto_writer_fails_closed_when_mixed_eol_has_no_local_context
  - crates/unica-xdto-designer/src/writer.rs::xdto_writer_removal_matrix_preserves_exact_outer_slices
  - crates/unica-xdto-designer/src/writer.rs::xdto_writer_sets_only_the_base_of_a_named_type
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_type_set_changes_only_the_base_and_repeats_the_current_base_as_no_op
---

# Изменение схемы XDTO сохраняет остальной текст пакета

Добавление и удаление типов и свойств, а также смена основы типа меняют
только выбранный `XDTOPackages/<Имя>/Ext/Package.bin`. Дескриптор объекта
метаданных остаётся отдельным ресурсом.

Пакет изменяется локально: байты за пределами правки, начальный BOM
и переводы строк сохраняются. Смена основы меняет только атрибут `base`
выбранного типа; если префикс основы не объявлен в области типа, рядом
с атрибутом повторяется его единственное объявление из пакета. Имя, свойства,
ограничения, прочие атрибуты типа и его место остаются прежними, прежняя
основа файла не меняет. Для вставки используется перевод строки
рядом с целью; при смешанном стиле без такого образца писатель отказывает.
Проверки проходят планировщик операций и писатель пакета.
