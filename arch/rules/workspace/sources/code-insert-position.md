---
id: INV.SOURCE.CODE-INSERT-POSITION
check:
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_without_a_selector_appends_to_the_end_and_proves_the_repeat
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::staged_code_args_report_exact_paths_and_reject_legacy_fields
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::insertion_around_an_addressed_method_needs_a_position_and_the_method
---

# Вставка без выбранного места добавляет код в конец модуля

Операция `code.insert` на адресе модуля или его `Body` без `selector`
добавляет текст в конец модуля и не принимает `position`. Место внутри
модуля выбирает текстовый якорь `selector.anchor`, и тогда позиция
обязательна: нужно указать, куда вставить текст относительно якоря.

Вставка до или после метода адресуется самим методом `…Method.<метод>`:
`position` (`before` или `after`) обязательна, `selector` не принимается.
Метод в `selector.method` снят и получает отказ с адресом метода.

Изменение текста проверено внутренним редактором BSL; сочетания аргументов —
текущим планировщиком. Эти проверки не заменяют полный вызов публичного `apply`.
