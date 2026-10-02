---
id: INV.APP.RECEIPT-RESULT-RESERVATION
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::promised_and_handoff_states_hold_worst_case_result_quota
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_bind_direct_ack_and_receipt_terminal_expiry_release_exact_quota
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::unbound_promise_terminal_keeps_canonical_payload_until_task_ttl
---

# Место для результата удерживается до передачи или завершения квитанции

Пока вызов принадлежит квитанции, для него учитывается место под наибольший
допустимый ответ — 8 МиБ + 64 КиБ. Переход к обещанному фоновому заданию
или начало его подготовки не освобождают этот резерв.

После подтверждённой передачи в TaskStore резерв квитанции освобождается;
связь с заданием сохраняется и переживает перезапуск. Для прямого ответа
резерв освобождает подтверждение, а для результата, оставшегося в квитанции
фонового задания, — истечение его срока хранения. Освобождение одного
резерва не уменьшает учёт других вызовов.

Конечный результат фонового задания, оставшийся в квитанции, хранится
весь срок хранения задания.
