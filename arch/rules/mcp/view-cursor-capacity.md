---
id: INV.APP.VIEW-CURSOR-CAPACITY
check:
  - crates/unica-coder/src/application/result_store.rs::cursor_chain_is_refused_before_it_can_exceed_the_entry_bound
  - crates/unica-coder/src/application/result_store.rs::exact_revision_change_is_stale_but_tampering_and_expiry_are_invalid
gap: https://github.com/IngvarConsulting/unica/issues/957
---

# Сохранённые страницы чтения ограничены памятью и сроком жизни

Страницы для продолжения `view` хранятся только в процессе сервера.
Хранилище ограничивает число записей, их общий размер и срок хранения;
при нехватке места вытесняются давно не читанные записи.
Цепочка, которая сама превышает вместимость, не публикуется как доступная.

Проверены отказ слишком длинной цепочке и истечение срока курсора.
Ограничение суммарных байтов и вытеснение требуют отдельных проверок
канонического хранилища; тесты старого `ResultStore` их не заменяют.
