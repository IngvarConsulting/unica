---
id: INV.APP.DAEMON-INVOCATION-HANDOFF
status: active
governs: product
decision: DEC.2026-09-08.DAEMON-V5-DEADLINE-OWNER
check:
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5.rs::complete_v5_frame_near_cutoff_cannot_receive_a_fresh_response_budget
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::cutoff_during_admission_projects_exact_unbound_task
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::response_budget_is_not_receipt_identity
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_hands_inline_work_over_the_cutoff_to_a_task_the_same_attempt_completes
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_hands_a_failing_inline_attempt_over_the_cutoff_and_the_task_fails_once
scope: [app]
---

# Седьмая секунда разделяет direct response и durable Task

Завершение до 7000 мс может вернуть direct DomainResult. В 7000 мс
незавершённая Invocation уже имеет durable Task; нулевой бюджет материализует её
до execution. Конкурентные completion и handoff публикуют один terminal result
при одном execution.

Подготовка result к transport не продлевает frontend deadline: разрешён один
заранее вычтенный запас 125 мс. Результат больше 16 KiB, завершившийся при
остатке не более этого запаса, материализуется как тот же durable Task без
повторного execution; малый результат в 6999 мс сохраняет direct-семантику.

Daemon захватывает один opaque absolute deadline своим executor clock сразу
после приёма request JSONL и до strict validation, actor admission/binding и
service preparation. Переданный frontend remaining budget может только сузить
его. Actor-bound/prepared invocation и response writer сохраняют тот же private
`Arc<Clock>` authority и те же границы; чужой clock с равными `Instant`
отклоняется до direct result, store и execution. Ни один этап daemon не
прибавляет duration к новому `now`.
