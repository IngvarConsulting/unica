---
id: INV.SOURCE.WRITE-CONTAINMENT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/path_policy.rs
scope: [source]
---

# Запись не выходит за корень рабочего пространства

Любой путь записи разрешается внутри рабочего пространства. Выход за корень — отказ,
а не предупреждение.
