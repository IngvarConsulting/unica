---
id: INV.RUNTIME.SERVICE-TOKEN
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::service_protocol_rejects_invalid_token_and_accepts_ping
---

# Внутренний сервис требует токен своего запуска

Workspace helper отклоняет запрос с чужим токеном. Токен относится
к конкретному запущенному сервису и хранится с его служебной записью.

Проверка проходит аутентификацию runtime и разрешённый `ping` напрямую.
Она не является проверкой защиты сетевого окружения или прав файловой системы.
