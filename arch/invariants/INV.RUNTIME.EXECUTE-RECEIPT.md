---
id: INV.RUNTIME.EXECUTE-RECEIPT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::runtime_execute_terminal_result_is_returned_in_original_call
scope: [app, product, wire]
---

# Синхронный runtime возвращает терминальную квитанцию в исходном вызове

Применённый `unica.runtime.execute` возвращает типизированный статус процесса,
код выхода и квитанцию исполнения тем же результатом операции, который получил
исходный вызов application.
