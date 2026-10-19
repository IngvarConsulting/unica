---
id: INV.APP.READ-CALL-JOINING
check:
  - crates/unica-coder/src/interfaces/daemon_router.rs::lost_submit_names_its_task_and_a_retry_joins_it_instead_of_running_again
  - crates/unica-coder/src/interfaces/daemon_router.rs::only_an_identical_read_joins_and_a_mutation_never_does
  - crates/unica-coder/src/interfaces/daemon_router.rs::an_external_edit_after_the_leader_started_makes_the_repeat_run_fresh
  - crates/unica-coder/src/interfaces/daemon_router.rs::one_holder_cancelling_leaves_the_shared_work_to_the_others_until_all_cancel
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_joined_holder_keeps_the_result_after_another_holder_cancelled
  - crates/unica-coder/src/interfaces/daemon_router.rs::holder_cancel_before_completion_stays_cancelled_at_the_same_epoch
  - crates/unica-coder/src/interfaces/daemon_router.rs::shared_completion_before_holder_cancel_keeps_result_at_the_same_epoch
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::a_stale_working_read_cannot_replace_an_observed_shared_terminal_with_cancelled
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::a_stale_alias_read_keeps_the_holder_cancellation_after_shared_completion
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::a_working_snapshot_from_before_the_cancel_request_must_reload
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::retired_or_foreign_holder_contexts_never_project_or_cancel_a_new_alias
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::reply_membership_capture_detects_attach_retire_aba_and_revision_overflow
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::an_unjoined_task_reply_does_not_read_its_canonical_store_again
  - crates/unica-coder/src/interfaces/daemon_router.rs::an_exact_repeat_of_a_call_with_its_own_receipt_never_becomes_a_joiner
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_frame_that_arrives_cancelled_never_leads_identical_reads
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::only_tools_that_change_nothing_are_joinable
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::only_the_exact_request_identity_finds_its_leader
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::a_mutation_closes_joins_to_reads_started_before_it
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::a_read_checked_before_a_mutation_never_leads_after_it
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::an_identical_read_joins_only_within_the_window_after_the_leader_started
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::shared_work_is_cancelled_only_when_every_holder_cancelled
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::the_joinable_index_and_the_holders_stay_bounded
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/join_freshness.rs::unchanged_sources_allow_the_join
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/join_freshness.rs::a_file_edited_after_the_walk_started_forbids_the_join
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/join_freshness.rs::a_file_edited_within_the_margin_before_the_walk_forbids_the_join
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/join_freshness.rs::an_edited_project_file_forbids_the_join
gap: https://github.com/IngvarConsulting/unica/issues/1743
---

# Одинаковое чтение присоединяется к идущему заданию

Новый вызов `view`, `resolve`, `search`, `check`, `diff` или `docs`
присоединяется к уже идущему заданию, если совпадают ядро демона, инструмент,
хеш нормализованных аргументов и область запроса (рабочее пространство).
Такой вызов не создаёт квитанции и не запускает обход повторно. Он получает
собственный `taskId` и видит общее задание под своей идентичностью;
результат общий для всех держателей. Вызов, у которого уже есть своя
квитанция, всегда читает её и держателем не становится.

Присоединение никогда не отдаёт результат старше исходников на момент
отправки присоединяющегося вызова. Вызов присоединяется, только если
с начала ведущего обхода не менялись `v8project.yaml` и ни один файл
и каталог объявленных наборов исходников: ни правкой через Unica, ни внешней
правкой. Проверка сравнивает время изменения содержимого и, где оно есть,
метаданных (`ctime`) с началом ведущего обхода за вычетом двух секунд
на грубость отметок файловой системы; служебный каталог `.build` не
учитывается. Ошибка чтения, превышение двух секунд или 500 000 записей
означают «менялись». Исключение одно: на Windows нет времени изменения
метаданных, и файл, которому восстановили прежнее время изменения
содержимого, не обнаруживается.

Сверх этого присоединение возможно только в течение 30 секунд после начала
ведущего чтения — около четырёх окон, за которые ошибка потерянного ответа
доходит до агента. `apply` и `run` не присоединяются никогда, включая
предварительный просмотр. Их отправка закрывает присоединение к чтениям,
начатым раньше, а пока они не завершились, новые чтения ведущими не
становятся.

Присоединение возможно только к заданию, которое не завершено и не отменено.
Ведущим становится только реально исполняемый вызов. Ведущий вызов внутри
своего окна прямого ответа к присоединению не предлагается; если его окно
уже истекло, новый вызов в пределах своего срока ждёт его перехода
в задание. Отмена одного держателя завершает только его вид задания отменой;
сама работа отменяется, когда её отменили все держатели. Завершение и сбой
общего задания видят все держатели.

Учёт держателей живёт в памяти демона и ограничен: при переполнении новый
вызов исполняется сам. Очистка временного держателя не разрешает удалять обещанный результат
или след дедупликации по возрасту. После перезапуска демона `taskId` присоединившегося
вызова неизвестен (`task_not_found`), а общее задание получает
восстановительный конечный статус.

Личный конечный вид первоначального держателя сохраняется отдельно от исхода
producer и переживает перезапуск по [правилу выдачи](execution-delivery.md).
Ранний Task lookup не меняет допустимость присоединения. Существующие проверки
отмены первоначального держателя подтверждают порядок в живом процессе;
долговечность его личного результата и единая authority состава держателей
остаются в `gap`.
