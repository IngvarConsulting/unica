---
id: INV.APP.DEPENDENCY-DIRECTION
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_rust_platform_boundary.py::test_repository_currently_complies_with_platform_boundary
scope: [app]
---

# Направление зависимостей между слоями закреплено проверкой

Домен не знает о вводе-выводе, application не знает о транспорте, инфраструктура не вызывает
application в обход портов. Нарушение ловится стражем, а не ревью.
