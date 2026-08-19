---
id: INV.PLATFORM.NO-ORPHANS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs
scope: [platform]
---

# Дочерние процессы удерживаются целыми деревьями

Порождённый процесс завершается вместе со своим деревом: осиротевший процесс переживает
сессию и держит рабочее дерево.
