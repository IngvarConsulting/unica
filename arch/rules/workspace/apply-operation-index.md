---
id: INV.APP.APPLY-OPERATION-INDEX
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::mixed_event_batch_refusal_names_the_original_operation
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/borrow.rs::mixed_borrow_batch_refusal_names_the_original_operation
gap: https://github.com/IngvarConsulting/unica/issues/964
---

# Ошибка применения указывает операцию исходного запроса

Если один запрос `apply` содержит операции разных семейств, ошибка
планирования указывает исходный индекс `ops[i]`. Разбиение запроса между
планировщиками не меняет этот индекс.

Проверки покрывают поздние отказы Event и Borrow в смешанных планах.
Они не доказывают сохранение индекса во всех семействах.
