---
id: INV.APP.SUPPORT-POLICY-READ-BOUNDS
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_warn_off_deny_database_and_malformed_match_v12
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_capture_stops_after_first_retained_read_chunk_write_free
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_all_absent_capture_rejects_terminal_cancellation_and_deadline_write_free
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_final_gate_stops_after_first_retained_read_chunk_and_rolls_back
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_second_pass_reuses_terminal_state_between_chunks
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_reader_preserves_limit_plus_one_in_64_kib_chunks
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_reader_retries_interrupted_after_partial_read
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_reader_stops_repeated_interrupts_at_terminal_state
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_reader_preserves_limit_plus_one_after_interrupt
---

# Чтение политики поддержки ограничено размером и сроком операции

Файл политики размером до 32 МиБ может разрешить `warn` или `off`.
Превышение этой границы даёт `deny`. Для обнаружения превышения читается
не более одного дополнительного байта.

Первое чтение и повторные проверки используют исходный срок и отмену
операции. До и после каждого блока размером не более 64 КиБ проверяется,
можно ли продолжать. Прерванное системное чтение повторяется с тем же
бюджетом и уже прочитанными байтами. Истечение срока или отмена останавливают
чтение, даже если оно повторяется или файл политики не найден.

До публикации такой отказ не меняет исходники и кеш; после публикации
вызывает откат. Это ограничение между системными вызовами: прерывание
одного зависшего вызова чтения не гарантируется.
