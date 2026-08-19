---
id: INV.APP.DEPENDENCY-DIRECTION
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: scripts/ci/check-rust-platform-boundary.py
scope: [app]
---

# Направление зависимостей между слоями закреплено проверкой

Домен не знает о вводе-выводе, application не знает о транспорте, инфраструктура не вызывает
application в обход портов. Нарушение ловится стражем, а не ревью.
