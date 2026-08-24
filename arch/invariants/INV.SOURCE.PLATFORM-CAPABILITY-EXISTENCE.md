---
id: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
status: active
governs: product
decision: DEC.2026-08-25.LOGICAL-TREE-CORE-SLICE
check: crates/unica-coder/src/infrastructure/logical_tree.rs::platform_capability_controls_logical_existence_without_filesystem_evidence
scope: [platform, source]
---

# Логическое существование модуля задаёт профиль платформы

Профиль 8.3.27 разрешает модуль по сочетанию владельца и семантической роли без
проверки файла. Допустимый нематериализованный модуль существует, а неизвестная
роль или недопустимое сочетание отвечает `not_found`. `Bot` и
`WebSocketClient` входят в профиль; HTTP, SOAP и сервис интеграции остаются
разными ролями. `RecordManager`, универсальный `Service` и gRPC отсутствуют.
