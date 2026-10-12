---
id: INV.APP.READ-CALL-JOINING
check:
  - crates/unica-coder/src/interfaces/daemon_router.rs::lost_submit_names_its_task_and_a_retry_joins_it_instead_of_running_again
  - crates/unica-coder/src/interfaces/daemon_router.rs::only_an_identical_read_joins_and_a_mutation_never_does
  - crates/unica-coder/src/interfaces/daemon_router.rs::one_holder_cancelling_leaves_the_shared_work_to_the_others_until_all_cancel
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_joined_holder_keeps_the_result_after_another_holder_cancelled
  - crates/unica-coder/src/interfaces/daemon_router.rs::an_exact_repeat_of_a_call_with_its_own_receipt_never_becomes_a_joiner
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::only_tools_that_change_nothing_are_joinable
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::only_the_exact_request_identity_finds_its_leader
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::a_mutation_closes_joins_to_reads_started_before_it
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::shared_work_is_cancelled_only_when_every_holder_cancelled
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/joining.rs::the_joinable_index_and_the_holders_stay_bounded
---

# Одинаковое чтение присоединяется к идущему заданию

Новый вызов `view`, `resolve`, `search`, `check`, `diff` или `docs`
присоединяется к уже идущему заданию, если совпадают ядро демона, инструмент,
хеш нормализованных аргументов и область запроса (рабочее пространство).
Такой вызов не создаёт квитанции и не запускает обход повторно. Он получает
собственный `taskId` и видит общее задание под своей идентичностью;
результат общий для всех держателей. Вызов, у которого уже есть своя
квитанция, всегда читает её и держателем не становится.

`apply` и `run` не присоединяются никогда, включая предварительный просмотр.
Их отправка закрывает присоединение к чтениям, начатым раньше, а пока они
не завершились, новые чтения не становятся общими. Внешние правки исходников
не отслеживаются: присоединившийся вызов получает результат обхода, начатого
до него, пока тот идёт.

Присоединение возможно только к заданию, которое не завершено и не отменено.
Ведущий вызов внутри своего окна прямого ответа к присоединению не
предлагается; если его окно уже истекло, новый вызов в пределах своего срока
ждёт его перехода в задание. Отмена одного держателя завершает только его вид
задания отменой; сама работа отменяется, когда её отменили все держатели.
Завершение и сбой общего задания видят все держатели.

Учёт держателей живёт в памяти демона и ограничен: при переполнении новый
вызов исполняется сам. Запись держателя удаляется вместе с общим заданием
по сроку его хранения. После перезапуска демона `taskId` присоединившегося
вызова неизвестен (`task_not_found`), а общее задание получает
восстановительный конечный статус.
