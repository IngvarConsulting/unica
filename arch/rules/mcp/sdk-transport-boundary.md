---
id: INV.WIRE.SDK-TRANSPORT
---

# MCP SDK принадлежит транспортному слою

Только Cargo-пакет `unica-coder-daemon` прямо зависит от `rmcp` из crates.io.
Зависимость не переименовывается, отключает default features и включает
ровно `server` и `transport-io`.

Продуктивные ссылки на `rmcp` во всех отслеживаемых Git Rust-файлах `src`
пакетов workspace находятся только в `crates/unica-coder-daemon/src/interfaces/`.
Комментарии, литералы и элементы с точным `#[cfg(test)]` в эту границу
не входят.

Производственная library-сборка подтверждает, что `UnicaServer` реализует
`rmcp::ServerHandler`: соответствие SDK не должно существовать только
в тестовой сборке. Эту сборку выполняет конвейер Rust, а не Python-набор:
ordinary-архив в каждом профиле и точный production-набор фич шага
`product` джобы `Unica CI` во всех кодовых прогонах, кроме pull request
и push в `main`.
