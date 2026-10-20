---
id: CTR.WIRE.NATIVE-TASK-PROJECTION
check:
  - crates/unica-coder/src/interfaces/mcp.rs::native_task_projection_contract_is_capability_gated_durable_and_replay_free
  - crates/unica-coder/src/interfaces/mcp.rs::native_task_get_reports_late_cancel_request_without_claiming_cancellation
  - crates/unica-coder/src/interfaces/task_projection.rs::v5_native_projection_keeps_durable_time_ttl_and_maps_queued_to_working
  - crates/unica-coder/src/interfaces/task_projection.rs::v5_completed_task_embeds_the_exact_direct_call_result
  - crates/unica-coder/src/interfaces/task_projection.rs::source_text_result_is_an_envelope_then_raw_text_without_structured_content
  - crates/unica-coder/src/interfaces/task_projection.rs::task_result_with_source_text_follows_the_client_budget
  - crates/unica-coder/src/interfaces/task_projection.rs::error_fallback_is_bounded_without_losing_structured_diagnostics
  - crates/unica-coder/src/interfaces/task_projection.rs::error_without_diagnostics_still_has_readable_summary
  - crates/unica-coder/src/interfaces/task_projection.rs::error_recovery_preserves_all_hints_beyond_the_diagnostic_text_budget
  - crates/unica-coder/src/interfaces/task_projection.rs::error_recovery_that_exceeds_the_total_projection_limit_is_refused_whole
  - crates/unica-coder/tests/v13_search_integration.rs::public_view_refuses_bsl_text_over_the_default_budget_whole_with_structural_next
  - crates/unica-coder/tests/v13_search_integration.rs::oversized_error_recovery_is_a_closed_size_refusal_without_poisoning_the_daemon
  - crates/unica-coder/src/application/receipt_ledger.rs::recovery_that_cannot_be_delivered_is_rejected_before_terminal_admission
  - crates/unica-coder/src/application/v13/source_text.rs::admission_refusal_bounds_client_specific_range_hints
  - crates/unica-coder/src/interfaces/mcp.rs::v13_compatibility_task_tools_are_profile_gated_durable_and_replay_free
  - crates/unica-coder/src/interfaces/task_projection.rs::v5_failed_and_cancelled_terminals_answer_only_the_closed_vocabulary
---

# Native Task сохраняет состояние и результат вызова

Клиент с [Tasks capability](native-task-capability.md) получает
`CreateTaskResult`, затем читает задание через `tasks/get`. Отмена
`tasks/cancel` идемпотентна. `tasks/update` сначала проверяет существование
и срок жизни задания, затем отвечает `task_input_not_supported`:
дополнительный ввод для выполняющегося задания не поддерживается.
После `CreateTaskResult` frontend не отправляет самопроизвольных сообщений
прогресса или опроса.

Идентификатор, время создания и обновления, TTL и интервал опроса приходят
из сохранённого состояния. Времена передаются в ISO-8601; обновление раньше
создания отклоняется. Встроенный протокол MCP представляет `queued`
как `working`. Неизвестный, истёкший и некорректный идентификаторы дают
различимые закрытые ошибки.

Если сохранён запрос отмены, `tasks/get` передаёт его через стандартное
`statusMessage`. Сообщение не заменяет состояние задания и фактический
результат; клиент MCP сам решает, показывать ли его модели.

Завершённое задание содержит тот же сериализованный `CallToolResult`, что
и прямой ответ тому же клиенту: канонический JSON находится в `structuredContent`,
при успехе `content` пуст. Исключение — успешный ответ с исходным текстом
[тела BSL](../workspace/sources/module-compilation-and-body.md): в нём нет
`structuredContent`, а `content` несёт два текстовых блока — сначала
канонический JSON без текста, затем сам текст UTF-8 без экранирования.
Хост, получивший `structuredContent`, может передать модели только его,
поэтому конверт и текст идут одним каналом. Сохранённый результат задания
и его дайджест включают текст. При `isError: true` `content` содержит текстовое
представление `summary` и `diagnostics`, чтобы клиент мог показать причину
отказа, даже если не читает `structuredContent`. Этот блок диагностики
занимает не более 4096 байт UTF-8 вместе с явным маркером усечения;
полные данные остаются в `structuredContent`. Непустой `next` передаётся
дополнительным текстовым JSON-блоком целиком: все варианты продолжения,
их аргументы и пояснения сохраняются без усечения. Поле `data` в текст
не копируется. Полные подсказки учитываются в общем пределе MCP ниже;
превышение отказывает всему ответу, а не выдаёт часть подсказок.
До сохранения нового результата проверяется и эта текстовая форма с запасом
для служебных полей Task. Проверка учитывает добавляемый диапазон Body для
любого поддерживаемого бюджета клиента; сам сохранённый результат от бюджета
клиента не зависит.
`isError`, `_meta` и `resultType` сохраняются.
Сериализованные `CallToolResult` и `DetailedTask` не превышают
8 MiB + 64 KiB; превышение даёт `result_too_large`.

Получение, обновление и отмена используют один абсолютный срок, не позднее
7000 мс + 125 мс от приёма запроса frontend. Подключение, handshake, отправка,
чтение и разбор ответа расходуют этот срок; отдельная фаза не начинает
новое окно ожидания.
