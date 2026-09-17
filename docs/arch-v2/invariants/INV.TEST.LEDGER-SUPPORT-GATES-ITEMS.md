---
id: INV.TEST.LEDGER-SUPPORT-GATES-ITEMS
status: active
governs: process
decision: DEC.2026-09-08.V5-RUNTIME-HOOKS
check: tests/ci/test_receipt_ledger_test_support_boundary.py::test_feature_attributes_gate_items_not_statements
scope: [ci, app]
---

# Признак тестовой поддержки ledger гейтит только элементы

В `crates/unica-coder/src` атрибут `cfg`, упоминающий признак
`receipt-ledger-test-support`, стоит только перед элементом модуля: `mod`,
`use`, `fn`, `struct`, `enum`, `impl`, `trait`, `type`, `const`, `static`.
Оператор, выражение, аргумент, поле, параметр, ветка `match` и блок под
признаком запрещены, как и любая форма `not(feature = …)` — у production
нет кода, который есть только без признака. Что рантайм показывает
наблюдателю и о чём спрашивает его, идёт через `V5RuntimeHooks`.
