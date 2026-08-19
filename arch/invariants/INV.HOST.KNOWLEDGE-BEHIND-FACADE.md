---
id: INV.HOST.KNOWLEDGE-BEHIND-FACADE
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_rust_platform_boundary.py::test_repository_currently_complies_with_platform_boundary
scope: [host]
---

# Знание о хосте живёт за host-фасадом

Оркестратор нейтрален к хосту; каталоги, переменные и соглашения конкретного хоста читает
только фасад, и добавление хоста не меняет мест вызова.
