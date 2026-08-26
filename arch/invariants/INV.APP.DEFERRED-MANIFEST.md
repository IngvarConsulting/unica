---
id: INV.APP.DEFERRED-MANIFEST
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::oversized_typed_read_returns_a_manifest_within_budget
scope: [app]
---

# Большое чтение отвечает ограниченным манифестом

Типизированный результат сверх порога заменяется успешным манифестом
`deferred` с `resultRef`, описанием секций и идентичностью снимка.
