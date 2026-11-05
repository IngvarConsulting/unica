---
id: INV.SOURCE.CODE-TEXT-SYNTAX-ERRORS
check:
  - crates/unica-coder/tests/platform/code_patch_composition.rs::a_text_that_breaks_the_module_names_every_syntax_error_by_its_place
  - crates/unica-coder/tests/platform/code_patch_composition.rs::a_module_that_did_not_parse_names_its_errors_as_the_source
  - crates/unica-execution-contract/src/source_text_budget.rs::refusal_diagnostics_fill_the_budget_and_count_the_rest
---

# Синтаксическая ошибка в тексте правки называет свои места

Если исходный модуль разбирается, а модуль после `code.replace` или
`code.insert` содержит синтаксическую ошибку, `apply` отказывает до записи
кодом `bad_value` с исходом `fixCall` по пути `ops[<i>].args.text`.
Сообщение называет первую ошибку: строку, столбец и суть.

За отказом следуют все синтаксические ошибки результата как предметные
диагностики в той же форме, что у `unica.check` и диагностик редактора:
код, сообщение, диапазон в модуле и диапазон в присланном тексте, если
ошибка в нём. Перечень ограничивает только
[бюджет ответа клиента](../../mcp/source-text-response-budget.md). Если
в бюджет вошли не все ошибки, ответ называет общий счёт и сколько показано,
и ни одна ошибка не пропадает молча.

Синтаксическая ошибка в модуле до правки остаётся отказом `invalid_source`
и перечисляет свои ошибки так же.
