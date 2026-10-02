---
id: INV.RUNTIME.SERVICE-REQUEST-FRAMING
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_request_lines_are_bounded_and_resynchronize
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_header_deadline_is_aggregate_across_drip_bytes
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_header_timeout_is_connection_local
gap: https://github.com/IngvarConsulting/unica/issues/978
---

# Незавершённый запрос не удерживает сервис бесконечно

Строка запроса внутреннего workspace-сервиса ограничена 8 МиБ. Превышение
отвергается; следующая строка разбирается отдельно. Чтение заголовка имеет
единый срок: поступление очередного байта не начинает его заново. Истечение
срока одного соединения не мешает другим соединениям.

Рабочий срок чтения заголовка — 5 секунд от принятия соединения.

Проверки используют настоящий ограниченный reader с управляемыми часами
и TCP-сервис с сокращённым сроком заголовка.
Привязка рабочего запуска к пяти секундам требует отдельной проверки.
