---
id: INV.WIRE.EOF-DRAINS-WORKERS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::eof_cleanup_drains_noncooperative_diagnostic_worker_within_the_same_grace
scope: [wire]
---

# EOF-дренирование не оставляет отслеживаемый worker

При EOF отслеживаемый диагностический worker, который не поддерживает
кооперативную отмену, завершается в пределах общего grace-периода очистки.
