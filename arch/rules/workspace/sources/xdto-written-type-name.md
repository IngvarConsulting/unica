---
id: INV.SOURCE.XDTO-WRITTEN-TYPE-NAME
check:
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_preserves_declared_prefixed_qnames_exactly
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_repeats_one_package_wide_binding_on_each_new_qname_element
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_leaves_unbound_qnames_for_validation_to_reject
  - crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs::xdto_writer_rejects_ambiguous_package_wide_prefix_identity
---

# При записи типа XDTO сохраняется указанное имя с префиксом

Писатель сохраняет переданное имя типа, например `self:Local`, без замены
префикса. Если объявление префикса находится вне места вставки, его можно
повторить на новом элементе только при единственном соответствии префикса
пространству имён во всём пакете. Необъявленный или неоднозначный префикс
вызывает диагностику валидатора; пространство имён не угадывается.

Проверки проходят писатель и валидатор пакета.
