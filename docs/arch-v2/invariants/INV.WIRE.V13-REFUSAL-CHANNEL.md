---
id: INV.WIRE.V13-REFUSAL-CHANNEL
status: active
governs: product
decision: DEC.2026-09-01.V0-13-REFUSAL-DISCIPLINE
check: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_refusals_answer_one_diagnostics_channel_from_the_closed_code_set
scope: [app, wire]
---

# Отказ v0.13 несёт закрытый код в diagnostics и не маскирует конфликт

Отказ канонического вызова отвечает `diagnostics[0]` с кодом из закрытого
множества и непустым `message`; `data` не несёт второго кода. Конфликт `ifRev`
отвечает `stale_revision` с обеими ревизиями. Отказ по отсутствующему обязательному
аргументу с закрытым доменом значений перечисляет этот домен. Допущенный логический
scope без исходного поддерева отвечает пустым результатом без диагностик, а не
отказом и не сырой ошибкой ОС.
