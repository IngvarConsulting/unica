---
id: INV.WIRE.TOOL-LIST-COMPATIBILITY
check:
  - crates/unica-coder/src/interfaces/mcp.rs::modern_direct_first_tools_list_pages_through_the_full_registry
  - crates/unica-coder/src/interfaces/mcp.rs::modern_tools_list_rejects_a_cursor_the_server_never_issued
  - crates/unica-coder/src/interfaces/mcp.rs::tools_list_rejects_any_presented_cursor
  - crates/unica-coder/src/interfaces/mcp.rs::modern_meta_inside_legacy_session_keeps_the_session_model
---

# Обход каталога инструментов зависит от версии запроса

Для версии `2026-07-28` `tools/list` отдаёт большой каталог страницами. Обход выданных
курсоров возвращает все имена без повторов; неверный курсор даёт `-32602`.
Ответ содержит `resultType: complete`. Настройки кеширования описаны
в [отдельном правиле](mcp-list-cache-fields.md).

Обычный запрос сессии `2025-11-25` получает весь каталог;
предъявленный ему курсор отклоняется. Современные метаданные отдельного
запроса меняют формат его ответа, но не переключают всю старую сессию.

Постраничный обход проверяется на большом тестовом реестре legacy.
Нынешний компактный каталог v0.13 не объявляется большим ради этой проверки.
