---
id: INV.PLATFORM.OS-BEHIND-FACADE
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_rust_platform_boundary.py::test_repository_currently_complies_with_platform_boundary
scope: [platform]
---

# Зависящий от ОС код живёт за платформенными фасадами

Ветвление по операционной системе допустимо только внутри платформенных адаптеров, у стража
нет исключений по путям, а тесты адаптера лежат рядом с ним.
