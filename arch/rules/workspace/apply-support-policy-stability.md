---
id: INV.APP.RETAINED-APPLY-SUPPORT-POLICY-EVIDENCE
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_absent_chain_rejects_nearer_policy_insertion_before_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_exact_file_rejects_byte_change_and_rename_replacement
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_stable_deny_evidence_allows_unrelated_dry_run_and_real_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_category_and_identity_transitions_are_rejected
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_candidate_parent_replacement_is_rejected
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_exact_and_oversized_reject_name_replacement_after_pre_read_identity
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_exact_rejects_name_replacement_after_retained_read_before_acceptance
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_dry_run_churn_is_write_free_and_returns_no_receipt
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_churn_after_source_publication_rolls_back_all_retained_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_same_inode_churn_during_late_final_gate_rolls_back_all_retained_state
---

# Применение изменений повторно проверяет разрешившую их политику

При допуске `apply` сохраняет выбранный `.v8-project.json` и отсутствие
предшествующих кандидатов. Режимы `warn` и `off` требуют обычного файла
с сохранёнными байтами и физической идентичностью. Недоступный, слишком
большой файл или объект другого вида даёт `deny`; это само по себе
не запрещает изменения, к которым блокировка поддержки не относится.

Перед публикацией, перед результатом `dryRun` и после записи подготовленных
данных вся сохранённая цепочка проверяется дважды. Она должна совпадать
с сохранёнными сведениями: отсутствиями, видами, физическими идентичностями
и точными байтами полностью прочитанного файла. Изменение до записи даёт отказ без публикации;
после записи восстанавливаются исходники, кеш и состояние ревизии.

Эта проверка обнаруживает сохраняющееся изменение. Она не доказывает,
что файл никогда не менялся между проверками, и не защищает от подмены
с возвратом прежнего состояния или от изменения после последней проверки.
Политика только читается; состав записывающих участников задан
[границей исходников и кеша](apply-source-cache-boundary.md).
