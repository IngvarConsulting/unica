---
id: INV.CI.EXACT-CARGO-CACHE
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_platform_build_uses_exact_cargo_cache_and_reports_outcome
scope: [ci]
---

# Кеш Cargo точен и наблюдаем

Ключ платформенной сборки включает ОС, цель, ключ тулчейна и `Cargo.lock`, не
использует префикс восстановления и сообщает исход кеша вместе с длительностями.
