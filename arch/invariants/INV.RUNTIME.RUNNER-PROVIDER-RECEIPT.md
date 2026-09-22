---
id: INV.RUNTIME.RUNNER-PROVIDER-RECEIPT
status: active
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check: crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::runner_011_provider_receipt_replaces_selection_for_all_three_operations
scope: [wire, product]
---

# CF/DT принимает квитанцию закреплённого раннера

Три операции CF/DT принимают provider receipt закреплённого раннера
без снятого selection. Старый список кандидатов не является входом контракта.
