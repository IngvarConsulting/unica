---
id: INV.PKG.PACKAGED-PUBLIC-SURFACE
status: active
governs: product
decision: DEC.2026-08-31.V0-13-SURFACE-FIRST-CUTOVER
check: crates/unica-bootstrap/tests/platform/verification_contract.rs::verify_requires_each_lifecycle_to_expose_each_public_tool
scope: [host, pkg, product, wire]
---

# Bootstrap проверяет два MCP lifecycle и точный compatibility-профиль

Проверка runtime требует успешный legacy `initialize` с последующим
`tools/list`, а также direct-first `server/discover` и `tools/list`. Оба списка
должны быть точно равны восьми предметным инструментам v0.13 плюс
`unica.task.get`, `unica.task.result`, `unica.task.cancel`. Отсутствующее новое
или оставшееся legacy-имя закрывает package gate. Аргументы, результаты и
предметное поведение проверяются отдельными правилами.
