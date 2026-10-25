---
id: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
check:
  - crates/unica-coder/src/application/v13/task_tools.rs::compatibility_contracts_are_exact_and_teach_bounded_wait
  - crates/unica-coder/src/application/v13/task_tools.rs::compatibility_parser_accepts_only_canonical_ids_and_wait_zero_through_7000
  - crates/unica-coder/src/application/v13/task_tools.rs::compatibility_receipt_is_closed_and_terminal_result_reuses_domain_json
  - crates/unica-coder/src/application/v13/task_tools.rs::late_cancel_request_is_visible_in_task_state_without_changing_subject_result
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_task_get_and_result_preserve_each_closed_failure_reason
  - crates/unica-coder/src/interfaces/mcp.rs::v13_compatibility_task_tools_are_profile_gated_durable_and_replay_free
  - crates/unica-coder/tests/daemon_receipt_ledger/frontend_restart.rs::compatibility_completed_task_survives_daemon_process_restart
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_get_and_cancel_do_not_replace_open_frontend_cutoff_with_125ms
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_get_and_cancel_share_one_absolute_cutoff_across_connect_and_exchange
  - crates/unica-daemon/src/client_v5.rs::task_exchange_post_parse_expiry_wins_for_valid_and_malformed_frames
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_result_keeps_original_cutoff_after_frontend_admission_delay
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_ready_result_at_wait_zero_survives_a_slow_connect
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_final_projection_obeys_cutoff_and_same_result_remains_recoverable
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_final_projection_preserves_exchange_failure_stage_and_generation
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_long_producer_survives_7000_and_100_ms_observations_once
  - crates/unica-coder/src/interfaces/mcp.rs::compatibility_recovers_same_task_after_partial_malformed_and_late_frames
  - crates/unica-daemon/src/task_observation.rs::observation_exposes_only_bounded_timing_and_generation_digest
  - crates/unica-coder/src/interfaces/mcp.rs::native_task_transport_failure_logs_the_observation_and_answers_only_the_refusal
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

Подключение и обмен расходуют один исходный срок frontend, у `result`
тоже. `waitMs + 125 мс` от получения запроса ограничивает только ожидание
демона; более ранний срок хоста сильнее. Прошедшее время, в том числе
на подключение, уменьшает это ожидание, но не срок обмена: готовый
результат при `waitMs: 0` приходит результатом и на загруженной машине,
где подключение дольше 125 мс. Поздний ответ
не публикуется, а его соединение закрывается для повторного использования.
Срок проверяется после разбора ответа: если он истёк при разборе,
корректность полученных данных уже не меняет причину отказа. Ещё раз срок
проверяется после построения `CallToolResult`: поздний snapshot не становится
своевременным результатом. Уже установленный отказ обмена сохраняет исходную
причину. Последующее оформление JSON-RPC и запись SDK при противодавлении
хоста не входят в этот checkpoint.

На проводе транспортный отказ — только закрытый код `task_transport_failed`,
и в совместимом, и в нативном ответе: он один решает, что агент делает
дальше. Сведения для разбора хоста уходят одной строкой в журнал frontend:
этап, на котором прервалось чтение, время от получения запроса до сбоя,
время начала и длительность прерванного шага, его cutoff и истёк ли он. Если
конкретный endpoint известен, его поколение обозначается хешем
идентификатора экземпляра. Подключение и handshake могут обозначаться общей
стадией получения peer; это не доказательство причины внутри одной из них.
Строка журнала не содержит токен endpoint, argv, приватные пути или сырой
текст IO/parse-ошибки.

Проверка сочетает MCP-вызовы с управляемыми обработчиками и сокетами
с настоящим перезапуском runtime v5 поверх сохранённого состояния.
