---
id: INV.RUNTIME.TIMERS-PRESERVE-WORK
check:
  - crates/unica-coder/src/domain/operation_deadline.rs::absence_and_elapsed_finite_deadline_are_distinct
  - crates/unica-coder/src/application/receipt_ledger_actor.rs::no_deadline_actor_queue_waits_for_running_port_then_returns_success
  - crates/unica-coder/src/application/receipt_ledger_actor.rs::no_deadline_mutation_keeps_late_successful_completion_authority
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::no_deadline_actor_returns_late_real_store_commit_and_reopens
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::no_deadline_row_commit_survives_delayed_sync_and_exact_reopen
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::no_deadline_keeps_visible_io_failure_commit_uncertain
  - crates/unica-coder/src/domain/code_intelligence.rs::provider_absence_is_distinct_from_expired_and_contracts_with_finite_deadlines
  - crates/unica-coder/src/domain/code_intelligence.rs::provider_finite_extreme_budgets_preserve_remaining_without_instant_overflow
  - crates/unica-coder/src/domain/code_intelligence.rs::provider_relative_equality_compares_the_same_endpoint_with_different_starts
  - crates/unica-coder/src/infrastructure/deadline_lock.rs::no_deadline_waits_on_occupied_lane_then_acquires_after_release
  - crates/unica-coder/src/infrastructure/deadline_lock.rs::no_deadline_occupied_lane_still_observes_explicit_cancellation
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::git_grep_without_deadline_passes_absence_to_the_process_runner
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::bsl_search_without_deadline_refuses_before_legacy_service_dispatch
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::bsl_graph_without_deadline_refuses_before_legacy_service_dispatch
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::rlm_search_without_deadline_refuses_before_legacy_service_dispatch
  - crates/unica-coder/src/infrastructure/rlm_navigation.rs::no_deadline_navigation_refuses_before_readiness_or_legacy_service_dispatch
  - crates/unica-coder/src/infrastructure/diagnostics.rs::no_deadline_diagnostics_refuses_before_legacy_backend_dispatch
  - crates/unica-coder/src/infrastructure/diagnostics.rs::no_deadline_diagnostics_explicit_cancel_wins_before_unsupported_bridge
  - crates/unica-coder/src/infrastructure/platform/source_revision_fence.rs::no_deadline_fsevents_flush_observes_write_and_preserves_explicit_cancel
  - crates/unica-coder/src/infrastructure/bsl_outline.rs::no_deadline_outline_reads_source_but_preserves_scope_and_explicit_cancel
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::no_deadline_task_commit_preserves_exact_terminal_cas_and_reopens
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::no_deadline_task_visible_io_failure_is_still_commit_uncertain
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::no_deadline_links_preserve_exact_cas_and_reopen_terminal_winner
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Таймер обслуживает работу, а ограничивающий срок требует основания

Истечение времени само по себе не доказывает ошибку операции или смерть
процесса. По умолчанию оно не отменяет принятую работу, не удаляет её результат
или след дедупликации и не лишает потребителя обещанного продолжения.
Ограничивающий срок требует того же основания, что
[ограничение ресурса](resource-limit-evidence.md), и явного решения о поведении
при его достижении. Произвольный таймаут не становится защитой от зависания
только потому, что назван watchdog.

Явное отсутствие срока отличается от истёкшего конечного срока. Ожидание
в очереди и позднее успешное завершение записи сами по себе не превращают
вызов без срока в ошибку или неопределённый исход. Отсутствие срока
не ослабляет проверку принадлежности записи и подтверждение её сохранения:
ошибка после видимого изменения по-прежнему требует точного восстановления.

Таймер допустим для следующих действий:

- Опрос состояния, пауза между попытками и backoff. Интервал не становится
  общим сроком операции или скрытым пределом числа попыток. Повтор сохраняет
  точный ключ; неопределённый исход не разрешает повторить внешний эффект.
- Ограниченное ожидание наблюдателя: возврат текущего статуса с возможностью
  продолжить наблюдение и получить результат. Принятая работа остаётся живой.
- Диагностика отсутствия прогресса и heartbeat. Сигнал сообщает наблюдение,
  но отсутствие heartbeat не доказывает завершение владельца.
- Освобождение неиспользуемого сервиса или восстанавливаемого кеша после
  проверки отсутствия выполняющейся и ожидающей работы и потребителей
  сохраняемого состояния. Перечитывание изменившихся исходников не
  восстанавливает обещанный снимок курсора.
- Завершение мягкой фазы после явно запрошенной остановки и переход к
  принудительной остановке принадлежащих операции процессов. Это не разрешает
  нарушить атомарность записи и восстановление. Истечение grace не доказывает
  выход процессов и не освобождает владение до фактического завершения.

Ручная отмена, прекращение ожидания и разрыв соединения различаются по
[правилу отмены](../mcp/explicit-call-cancellation.md). Таймер не подменяет
эти события. Размеры порций и интервалы опроса не ограничивают общий объём
работы. Тестовые watchdog и измеряемые цели производительности не являются
продуктовыми сроками операций.

В #1119 остаётся пересмотр автоматических отсечек, описанных в
[сроке сервиса](service-lifetime.md),
[сроке обращения к сервису](service-request-deadline.md) и
[жизненном цикле дерева процессов](../platform/process-tree-lifecycle.md).
Прежние нормативные требования к отсечкам пересматриваются по уточнённому
решению владельца. Согласование этих записей, кода и содержательных проверок
остаётся в #1119; старые формулировки не являются исключениями из нового
обязательства. Совместимость форматов и восстановление принятой работы
проверяются при изменении соответствующего пути. Снятие таймерного истечения
курсора само по себе не обещает сохранение курсора после перезапуска процесса.
