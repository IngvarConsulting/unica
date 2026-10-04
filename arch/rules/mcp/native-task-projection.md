---
id: CTR.WIRE.NATIVE-TASK-PROJECTION
check:
  - crates/unica-coder/src/interfaces/mcp.rs::native_task_projection_contract_is_capability_gated_durable_and_replay_free
  - crates/unica-coder/src/interfaces/mcp.rs::native_task_get_reports_late_cancel_request_without_claiming_cancellation
  - crates/unica-coder/src/interfaces/task_projection.rs::v5_native_projection_keeps_durable_time_ttl_and_maps_queued_to_working
  - crates/unica-coder/src/interfaces/task_projection.rs::v5_completed_task_embeds_the_exact_direct_call_result
  - crates/unica-coder/src/interfaces/task_projection.rs::error_fallback_is_bounded_without_losing_structured_diagnostics
  - crates/unica-coder/src/interfaces/task_projection.rs::error_without_diagnostics_still_has_readable_summary
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
и прямой ответ: канонический JSON находится в `structuredContent`,
при успехе `content` пуст. При `isError: true` он содержит текстовое
представление `summary` и `diagnostics`, чтобы клиент мог показать причину
отказа, даже если не читает `structuredContent`. Текст занимает не более
4096 байт UTF-8 вместе с явным маркером усечения; полные данные остаются
в `structuredContent`. Поле `data` в текст не копируется.
`isError`, `_meta` и `resultType` сохраняются.
Сериализованные `CallToolResult` и `DetailedTask` не превышают
8 MiB + 64 KiB; превышение даёт `result_too_large`.

Получение, обновление и отмена используют один абсолютный срок, не позднее
7000 мс + 125 мс от приёма запроса frontend. Подключение, handshake, отправка,
чтение и разбор ответа расходуют этот срок; отдельная фаза не начинает
новое окно ожидания.
