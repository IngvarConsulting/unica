---
id: INV.WIRE.SDK-INITIALIZE
check:
  - crates/unica-coder/src/interfaces/mcp.rs::initialize_uses_single_public_server_name_and_negotiates_version
---

# Initialize сообщает имя и версию сервера

При открытии соединения через `initialize` сервер сообщает имя `unica`
и версию Cargo-пакета. Версию протокола выбирает
[контракт согласования](protocol-negotiation.md).
