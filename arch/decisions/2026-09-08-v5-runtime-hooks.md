---
id: DEC.2026-09-08.V5-RUNTIME-HOOKS
status: active
governs: product
realized: tests/ci/test_receipt_ledger_test_support_boundary.py::test_feature_attributes_gate_items_not_statements
supersedes: []
superseded-by: null
establishes: [INV.TEST.LEDGER-SUPPORT-GATES-ITEMS]
design: docs/design/2026-09-08-daemon-v5-runtime-debt-design.md
---

# Рантайм v5 наблюдаем через один объект крючков, а не через признак сборки

**Решение.** Рантайм протокола v5 сообщает о своих шагах и спрашивает
разрешения на впрыснутые решения через один объект `V5RuntimeHooks`:
события, вход в стадии, точки пауз, отказы валидации, admission и prepare,
сбои хранилища, разрыв ответа, владение cutoff и подмены бегунка. Список
событий и точек пауз — закрытые production-типы. Production ставит
`NoHooks`, у которого каждый метод пуст; контракт ledger под признаком
`receipt-ledger-test-support` ставит свою реализацию поверх телеметрии и
сценарного управления. Слоты впрыска сбоев хранилищ (`TaskStore`,
`ReceiptLedger`) существуют всегда и заряжаются только крючками.
Признак сборки не ветвит production: атрибут `cfg`, упоминающий его, стоит
только на элементах модуля — `mod`, `use`, `fn`, `struct`, `enum`, `impl`,
`trait`, `type`, `const`, `static`, — а форма `not(feature = …)` запрещена.
Отказ реестра рабочих пространств при admission переводит daemon в
fail-stop и в production, как это делал впрыснутый отказ бегунка.

**Почему.** До этого в `runtime_v5.rs` стоял 261 атрибут признака, из них
сто с лишним на операторах и выражениях, а `#[cfg(not(feature))] let
owns_cutoff = true;` означал, что под признаком тестировался не тот код,
который отгружается: владение cutoff, fail-stop при отказе реестра и путь
завершения при перезапуске различались между сборками.

**Цена.** Виртуальный вызов пустого метода на каждом крючке — наносекунды
против десятков миллисекунд на команду ledger; закрытые списки событий и
пауз живут в production-коде и растут вместе с бегунком.
