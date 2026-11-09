---
id: INV.WIRE.SDK-MODULE-EXPORTS
---

# Транспортный модуль сохраняет публичный Rust API

Публичный API корневого файла `crates/unica-mcp/src/lib.rs` состоит из
`UnicaServer`, `tool_definitions()` и `run_stdio`. Функция `run_stdio`
принимает один параметр — функцию подключения к демону
`fn() -> Result<V5DaemonClient, String>`, которую передаёт `unica-product`;
она не имеет обобщений и возвращает `()`.

Корневой файл не добавляет других публичных сущностей и полей, элементов
с ограниченной видимостью (`pub(crate)` и подобных), публичных re-export и type alias,
вложенных public functions или экспортируемых макросов. Элементы с точным
атрибутом `#[cfg(test)]` или `#[cfg(any(test, feature = "test-support"))]`
исключены из проверки этой границы.
