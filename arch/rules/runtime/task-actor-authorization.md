---
id: INV.APP.DAEMON-ACTOR-AUTHORITY
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::bound_task_start_rejects_missing_foreign_stale_actor_proof_without_mutation
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::bound_task_start_rechecks_proof_after_working_readback
---

# Фоновая задача повторно подтверждает право актора перед исполнением

Запуск связанной с актором задачи требует подтверждения для её точного
вызова, записи задачи и версии связи с квитанцией. Отсутствующее, чужое
или устаревшее подтверждение отклоняется до изменения хранилищ,
подготовки и исполнения операции.

После сохранения и чтения состояния `working` право проверяется снова.
Если оно уже устарело, операция не начинается, а демон запрашивает
перезапуск. Восстановление завершает такую задачу как `interrupted`.

Проверки самого экземпляра актора, корня и ревизии описаны в
[правиле права чтения и публикации](../workspace/actor-source-authority.md).
