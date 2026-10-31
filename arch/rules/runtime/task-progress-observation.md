---
id: CTR.RUNTIME.TASK-PROGRESS-OBSERVATION
check:
  - crates/unica-execution/src/receipt_runtime/progress.rs::phases_are_not_poll_heartbeats_and_late_reporters_cannot_republish
  - crates/unica-execution/src/receipt_runtime/progress.rs::unwind_and_inline_attempts_leave_no_observation
  - crates/unica-coder/src/interfaces/mcp.rs::silent_extension_runner_progress_and_cancel_are_visible_over_public_mcp
  - tests/ci/test_acceptance_scenarios.py::AcceptanceFaultCorpusRunTests.test_silent_runner_task_phase_and_cancellation_over_public_mcp
---

# Наблюдение этапа не меняет состояние задания

Во время исполнения `extensions.list` задание передаёт известный Unica этап
и время его начала, длительность исполнения и текущего этапа. Ожидание раннера
не приписывает платформе внутренний этап, которого Unica не наблюдала.
Наблюдение содержит только этап из закрытого перечня и время; в него
не попадают вывод провайдера, аргументы, пути или учётные данные.
Повторное сообщение того же этапа и опрос задания не обновляют время его начала.

Наблюдение относится к активному исполнению и доступно только вместе с
его заданием. Оно не сохраняется как результат или состояние Task и не меняет
сохранённые время обновления, версию, TTL и интервал опроса. После завершения
или перезапуска отсутствие наблюдения означает отсутствие сведений об этапе.
Оно не доказывает зависание или продолжение работы процесса.

Запрос отмены и конечное состояние сохраняют
[прежнюю семантику](runner-mutation-cancellation.md). Наблюдение этапа
не подтверждает завершение дочерних процессов: такой факт устанавливает
владелец процесса, а не запрос отмены или время последнего сообщения.
