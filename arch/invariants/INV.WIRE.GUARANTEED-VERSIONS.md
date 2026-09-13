---
id: INV.WIRE.GUARANTEED-VERSIONS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-bootstrap/tests/platform/verification_contract.rs::verify_rejects_discover_without_the_guaranteed_versions
scope: [wire]
---

# Релизная проверка требует гарантированные версии протокола

Проверка установленного runtime отклоняет `server/discover`, если ответ не
содержит весь набор гарантированных версий протокола.
