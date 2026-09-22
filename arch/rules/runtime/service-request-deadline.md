---
id: INV.PERF.SERVICE-OPERATION-DEADLINE
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::cancellable_connector_cancel_control_uses_one_aggregate_500ms_budget
  - crates/unica-coder/src/infrastructure/workspace_services.rs::control_flush_success_cannot_cross_aggregate_500ms_budget
  - crates/unica-coder/src/infrastructure/workspace_services.rs::service_request_kind_deadline_matrix_is_exhaustive
---

# Обращение к внутреннему сервису имеет единый срок ожидания

Менеджер сервиса сохраняет один крайний срок на всю операцию. Чтение ответа
по частям и повтор после транспортной ошибки используют оставшееся время,
а не начинают отсчёт заново. Поступление очередного байта не позволяет
удерживать вызов бесконечно.

Подключение для каждого вида управляющего запроса дополнительно ограничено
500 мс. Недоступный сервис не оставляет такое подключение
в неограниченном ожидании.

Отправка отмены имеет отдельный общий срок 500 мс на подключение, запись
и flush; ответа она не ожидает.
