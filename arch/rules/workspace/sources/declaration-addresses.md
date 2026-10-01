---
id: INV.SOURCE.DECLARATION-BRANCHES-FOLLOW-THE-NAME
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::root_declarations_get_branches_and_only_the_named_one_is_addressable
---

# Именованная декларация читается по собственному адресу

Стандартная табличная часть читается по имени через `StandardTabularSection`
и объявляет вложенную ветвь `StandardAttribute` со своими стандартными
реквизитами.

У характеристики нет прикладного имени: коллекция `Characteristic`
возвращает строки данных без `at`. Попытка прочитать характеристику
по выдуманному имени завершается `not_found`.
