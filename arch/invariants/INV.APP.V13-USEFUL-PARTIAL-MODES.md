---
id: INV.APP.V13-USEFUL-PARTIAL-MODES
status: active
governs: product
decision: DEC.2026-08-31.V0-13-SURFACE-FIRST-CUTOVER
check: crates/unica-coder/src/infrastructure/daemon/server.rs::production_v3_daemon_configuration_executes_useful_modes_for_all_eight_v13_tools
scope: [app, product, wire]
---

# Каждый из восьми предметных инструментов имеет честный полезный режим

Production v3 daemon исполняет хотя бы один полезный замкнутый режим каждого из
восьми инструментов. Известные, но ещё не реализованные варианты не маскируются
как недоставленный provider и возвращают соответствующий typed `unsupported_*`.
Вариант чтения файлов рабочего пространства не считается полезным режимом, пока
он не проходит через actor-owned nofollow/cancellation capability; поэтому
`docs(source="configuration-documentation")` до такого адаптера отвечает
`unsupported_source`.
