---
id: INV.APP.DAEMON-RESULT-SIZE
check:
  - crates/unica-coder/src/application/receipt_ledger.rs::canonical_completed_terminal_rejects_result_over_eight_mib
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::oversized_result_and_uncertain_store_commit_fail_closed
  - crates/unica-coder/src/interfaces/mcp.rs::tasks_projection_bounds_near_limit_wire_and_rejects_over_limit
gap: https://github.com/IngvarConsulting/unica/issues/985
---

# Размер результата ограничен одинаково для прямого ответа и задачи

Сериализованный канонический результат операции не превышает 8 МиБ.
Превышение превращается в закрытую ошибку `result_too_large`; слишком большой
результат не выдаётся как успешный ответ и не сохраняется как результат успеха.

Передача вызова в задание не уменьшает допустимый размер: результат ровно
8 МиБ остаётся допустимым. Служебные поля ответа имеют отдельный запас;
он не увеличивает лимит самого результата. Граница MCP описана в
[правиле представления задачи](../mcp/native-task-projection.md).

До первого сохранения подготовленного конечного результата проверяется,
что он поместится и в последующие обязательные формы: запись задания,
связь с квитанцией и ответ. Сохранённый результат нельзя позднее заменить
`result_too_large` из-за роста служебных полей при переносе или восстановлении.
Полная проверка этого переноса указана в `gap`.
