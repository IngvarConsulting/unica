---
id: INV.SOURCE.IDEMPOTENT-PRECHECK
check:
  - crates/unica-coder/tests/platform/code_patch_composition.rs::patch_rejects_content_that_would_break_anchor_idempotence
  - crates/unica-coder/tests/platform/code_patch_composition.rs::patch_rejects_content_that_duplicates_the_selected_method
  - crates/unica-coder/tests/platform/code_patch_composition.rs::code_patch_replace_that_consumes_its_selector_cannot_apply_twice
  - crates/unica-coder/tests/platform/code_patch_composition.rs::whole_method_replacement_extracts_helpers_and_repeats_as_noop
  - crates/unica-coder/tests/platform/code_patch_composition.rs::inline_tail_after_the_method_is_not_a_fixed_point
---

# Повтор правки BSL не применяет то же изменение второй раз

Перед вставкой BSL проверяется, что следующий идентичный вызов распознает
уже добавленный текст и ничего не запишет. Если вставка сделает выбранный
метод или текстовый фрагмент неоднозначным, первый вызов отклоняется
без изменения файла.

Замена может переименовать выбранный метод или убрать выбранный фрагмент.
Тогда повторный вызов отказывает, потому что больше не находит цель,
и сохраняет результат первого применения.

Замена целого метода текстом из него самого и выделенных из него методов
считается уже выполненной, когда весь этот текст целыми строками стоит
в модуле на месте метода, поэтому повторный вызов ничего не запишет.
