---
id: INV.SOURCE.BOUND-PREIMAGES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::exact_read_guard_serializes_with_owner_writer_and_rejects_stale_plan
scope: [source]
---

# Мутация привязана к байтам, из которых выведена

Транзакция привязывает наблюдённые байты к себе как точные преобразы и
использует общие кооперативные блокировки публикации. Расхождение байтов между
планированием и публикацией отклоняет изменение.
