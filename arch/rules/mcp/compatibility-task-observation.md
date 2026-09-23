---
id: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
check:
  - crates/unica-coder/src/application/v13/task_tools.rs::compatibility_contracts_are_exact_and_teach_bounded_wait
  - crates/unica-coder/src/application/v13/task_tools.rs::compatibility_parser_accepts_only_canonical_ids_and_wait_zero_through_7000
  - crates/unica-coder/src/application/v13/task_tools.rs::compatibility_receipt_is_closed_and_terminal_result_reuses_domain_json
  - crates/unica-coder/src/application/v13/task_tools.rs::late_cancel_request_is_visible_in_task_state_without_changing_subject_result
  - crates/unica-coder/src/interfaces/mcp.rs::v13_compatibility_task_tools_are_profile_gated_durable_and_replay_free
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_get_and_cancel_do_not_replace_open_frontend_cutoff_with_125ms
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_get_and_cancel_share_one_absolute_cutoff_across_connect_and_exchange
gap: https://github.com/IngvarConsulting/unica/issues/928
---

# Compatibility Task возвращает сохранённое состояние без нового исполнения

В [режиме совместимости](mcp-tool-profiles.md) `unica.task.get` читает
задание сразу, `unica.task.result` ждёт его результат, `unica.task.cancel`
повторяет отмену без нового исполнения предметной операции. Все три
принимают канонический `taskId`; `result` допускает `waitMs` от 0 до 7000,
по умолчанию 7000. Некорректный идентификатор или срок, неизвестное,
истёкшее, неуспешное и отменённое задание дают различимые закрытые ошибки
через `diagnostics`.

Незавершённое задание возвращается в `structuredContent.data.task`
с пустым `content`, без сущностей `job` и `work`; пустые необязательные поля
опускаются. После перезапуска демона
сохраняются идентификатор, статус, времена и TTL; completed-задание
возвращает те же байты результата. Конечный `CallToolResult` совпадает
с прямым ответом той же предметной операции.
Если отмена запрошена, состояние задания содержит `cancelRequested: true`;
терминальный результат предметной операции от этого не меняется.

Подключение и обмен расходуют один исходный срок frontend. У `result`
он дополнительно ограничен `waitMs + 125 мс`; более ранний срок хоста
сильнее. Прошедшее время уменьшает ожидание демона. Поздний ответ
не публикуется, а его соединение закрывается для повторного использования.
Срок проверяется и после разбора ответа: если он истёк при разборе,
корректность полученных данных уже не меняет причину отказа.
Проверка этого случая для корректного и некорректного ответа пока отсутствует;
разрыв учтён в `gap`.

Проверка сочетает MCP-вызовы с управляемыми обработчиками и сокетами
с настоящим перезапуском runtime v5 поверх сохранённого состояния.
