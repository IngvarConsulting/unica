---
id: INV.PERF.RELEASE-ASSET-VERIFICATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_the_release_checks_every_address_it_publishes
scope: [ci, pkg, product]
---

# Релиз проверяет опубликованные байты и адреса поставки

Workflow повторно проверяет опубликованное ядро, достижимость всех адресов
движков и один полный `prefetch` перед продвижением поставки.
