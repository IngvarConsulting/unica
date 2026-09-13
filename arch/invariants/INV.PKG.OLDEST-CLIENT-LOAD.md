---
id: INV.PKG.OLDEST-CLIENT-LOAD
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_release_gate_pins_the_oldest_supported_client
scope: [pkg, product]
---

# Пакет проверяется нижней поддерживаемой версией клиента

Релизный шлюз устанавливает Claude Code `2.1.69`, проверяет фактическую версию
клиента и только затем валидирует пакет и каталог.
