---
id: INV.SOURCE.CODE-INSERT-POSITION
check:
  - crates/unica-coder/tests/platform/code_patch_composition.rs::code_patch_without_a_selector_appends_to_the_end_and_proves_the_repeat
  - crates/unica-coder/tests/platform/code_patch_composition.rs::staged_code_args_report_exact_paths_and_reject_legacy_fields
  - crates/unica-coder/tests/platform/code_patch_composition.rs::insertion_around_an_addressed_method_needs_a_position_and_the_method
  - crates/unica-coder/tests/platform/code_patch_composition.rs::insertion_around_a_method_keeps_its_head_together_and_respects_its_preprocessor
  - crates/unica-coder/tests/platform/code_patch_composition.rs::a_comment_inserted_before_a_method_joins_its_head_and_repeats_as_noop
---

# Вставка без выбранного места добавляет код в конец модуля

Операция `code.insert` на адресе модуля или его `Body` без `selector`
добавляет текст в конец модуля и не принимает `position`. Место внутри
модуля выбирает текстовый якорь `selector.anchor`, и тогда позиция
обязательна: нужно указать, куда вставить текст относительно якоря.

Вставка до или после метода адресуется самим методом `…Method.<метод>`:
`position` (`before` или `after`) обязательна, `selector` не принимается.
Метод в `selector.method` снят и получает отказ с адресом метода.
`before` вставляет текст перед головой метода — его документирующим
комментарием, аннотациями и директивами компиляции, — `after` — после
строки окончания. Условие препроцессора, которое обрамляет только этот
метод (`#Если … Тогда` стоит прямо над головой, `#КонецЕсли` — прямо под
окончанием), вставка обходит целиком вместе со строками комментария прямо
над ним; вложенные условия, обрамляющие только этот метод, обходятся все.
Если в условии есть и другой код или между условием и головой метода стоит
пустая строка, текст встаёт внутри условия рядом с методом: новый код
не выходит из контекста компиляции метода и не меняет чужой. Строки
комментария в конце вставленного перед методом текста становятся началом
его документирующего комментария; повтор той же вставки ничего не пишет.

Изменение текста проверено внутренним редактором BSL; сочетания аргументов —
текущим планировщиком. Эти проверки не заменяют полный вызов публичного `apply`.
