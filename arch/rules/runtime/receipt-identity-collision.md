---
id: INV.APP.RECEIPT-IDENTITY-COLLISION
check:
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::invocation_id_collision_rejects_before_any_mutation
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::reserved_task_id_collision_rejects_before_any_mutation
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::persisted_dual_index_collision_fails_reopen_before_temporary_cleanup_or_mutation
gap: https://github.com/IngvarConsulting/unica/issues/985
---

# Один идентификатор не связывает разные сохранённые вызовы

Пока сведения о вызове хранятся, его `invocationId` и зарезервированный
`taskId` принадлежат одному полному ключу квитанции. Новый ключ, совпавший
только по одному идентификатору, отклоняется до изменения хранилища.

Если противоречивые записи обнаружены при открытии, хранилище сообщает
о повреждении и не выбирает победителя. Даже очистка временных файлов
не должна изменить сведения, необходимые для разбора повреждения.

Проверки вводят коллизии в активные записи. Они не доказывают все сочетания
активной квитанции, подтверждённого ответа и связи с заданием.
