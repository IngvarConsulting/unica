---
id: INV.SOURCE.IDEMPOTENT-PRECHECK
check:
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::patch_rejects_content_that_would_break_anchor_idempotence
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::patch_rejects_content_that_duplicates_the_selected_method
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_replace_that_consumes_its_selector_cannot_apply_twice
---

# Повтор правки BSL не применяет то же изменение второй раз

Перед вставкой BSL проверяется, что следующий идентичный вызов распознает
уже добавленный текст и ничего не запишет. Если вставка сделает выбранный
метод или текстовый фрагмент неоднозначным, первый вызов отклоняется
без изменения файла.

Замена может переименовать выбранный метод или убрать выбранный фрагмент.
Тогда повторный вызов отказывает, потому что больше не находит цель,
и сохраняет результат первого применения.
