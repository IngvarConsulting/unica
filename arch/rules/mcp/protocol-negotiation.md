---
id: INV.WIRE.VERSION-TIERS
check:
  - crates/unica-coder/src/interfaces/mcp.rs::legacy_offer_2025_11_25_is_echoed
  - crates/unica-coder/src/interfaces/mcp.rs::legacy_unknown_offer_falls_back_to_pinned_version
  - crates/unica-coder/src/interfaces/mcp.rs::modern_discover_can_open_the_connection
  - crates/unica-coder/src/interfaces/mcp.rs::discovery_probe_response_matches_unwrapped_sdk_bytes
  - crates/unica-coder/src/interfaces/mcp.rs::discover_probe_followed_by_legacy_initialize_serves_compatibility_tools
  - crates/unica-coder/src/interfaces/mcp.rs::queued_ping_discovery_and_legacy_initialize_keep_every_frame
  - crates/unica-coder/src/interfaces/mcp.rs::unsupported_discovery_version_still_uses_sdk_error
  - crates/unica-coder/src/interfaces/mcp.rs::modern_unknown_version_direct_first_gets_unsupported_error
  - crates/unica-coder/src/interfaces/mcp.rs::modern_partial_meta_opener_is_rejected_before_serving
---

# MCP согласует версию и отклоняет неполное открытие соединения

Сервер объявляет ровно три версии: `2025-06-18`, `2025-11-25` и `2026-07-28`.
Неизвестное предложение в `initialize` получает запасную `2025-11-25`;
обновление SDK само по себе её не меняет.

Современный клиент может открыть соединение через `server/discover`
с полными служебными метаданными. Неизвестная версия прямого первого запроса
даёт `-32022` со сведениями о поддержке. Неполные метаданные такого запроса
не допускаются к обработчику.

Предварительный `server/discover` не закрепляет режим соединения: следующий
`initialize` выбирает legacy-сессию, а следующий прямой запрос с современными
метаданными остаётся в современном режиме.

Проверки проходят настоящий MCP-транспорт с тестовым реестром инструментов.
Состав и поведение предметного каталога они не подтверждают.
