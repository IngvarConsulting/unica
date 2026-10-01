---
id: INV.SOURCE.CODE-INSERT-POSITION
check:
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_without_a_selector_appends_to_the_end_and_proves_the_repeat
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::staged_code_args_report_exact_paths_and_reject_legacy_fields
---

# Вставка без выбранного места добавляет код в конец модуля

Операция `code.insert` без `selector` добавляет текст в конец модуля
и не принимает `position`. При наличии `selector` позиция обязательна:
нужно указать, куда вставить текст относительно выбранного места.

Изменение текста проверено внутренним редактором BSL; сочетания аргументов —
текущим планировщиком. Эти проверки не заменяют полный вызов публичного `apply`.
