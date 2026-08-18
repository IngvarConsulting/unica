---
id: INV.PLATFORM.OS-BEHIND-FACADE
status: active
decision: DEC.2026-08-18.CARRIED-RULES
check: scripts/ci/check-rust-platform-boundary.py
scope: [platform]
---

# Зависящий от ОС код живёт за платформенными фасадами

Ветвление по операционной системе допустимо только внутри платформенных адаптеров, у стража
нет исключений по путям, а тесты адаптера лежат рядом с ним.
