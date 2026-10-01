---
id: INV.WIRE.SDK-INITIALIZE
check:
  - crates/unica-coder/src/interfaces/mcp.rs::initialize_uses_single_public_server_name_and_negotiates_version
  - crates/unica-coder/src/interfaces/mcp.rs::legacy_unknown_offer_falls_back_to_pinned_version
---

# Initialize сохраняет имя сервера и предсказуемо согласует версию MCP

При открытии соединения через `initialize` сервер сообщает имя `unica`
и версию Cargo-пакета. Предложение протокола `2025-06-18` получает ту же
версию в ответе. Неизвестное предложение получает `2025-11-25`;
обновление SDK не должно самопроизвольно менять этот запасной вариант.

Проверки используют legacy-профиль инструментов. Сам обработчик
`initialize` общий для него и производственного профиля v0.13.
