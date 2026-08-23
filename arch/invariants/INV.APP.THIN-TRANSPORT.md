---
id: INV.APP.THIN-TRANSPORT
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_rust_platform_boundary.py::test_repository_currently_complies_with_platform_boundary
scope: [app]
---

# Транспорт только отображает протокол на вызовы application

Диспетчеризация и доменные решения принадлежат слою application; интерфейсный слой переводит
кадры протокола в вызовы и обратно, не принимая решений.
