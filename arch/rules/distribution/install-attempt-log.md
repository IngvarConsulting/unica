---
id: INV.PKG.INSTALL-ATTEMPT-LOG
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::an_attempt_is_already_open_while_the_download_runs
  - crates/unica-bootstrap/tests/runtime_install.rs::a_finished_install_leaves_nothing_unfinished
  - crates/unica-bootstrap/tests/runtime_install.rs::a_failure_the_bootstrap_reported_is_not_repeated_by_the_next_session
  - crates/unica-bootstrap/src/attempt.rs::an_attempt_is_discoverable_while_its_stage_runs
  - crates/unica-bootstrap/src/attempt.rs::a_new_attempt_does_not_erase_the_one_nobody_closed
  - crates/unica-bootstrap/src/attempt.rs::the_received_volume_is_read_from_the_partial_on_disk
  - crates/unica-bootstrap/src/attempt.rs::the_stage_reported_is_the_last_one_the_attempt_reached
---

# Незавершённая установка остаётся в журнале попыток

Bootstrap открывает запись об установке до начала загрузки. Журнал называет
артефакт, версию, целевую платформу и последнюю записанную стадию. Объём уже
полученных данных берётся из размера частичного архива на диске.

Успешная установка и обработанный отказ закрывают свою попытку. Незакрытая
попытка остаётся доступной при чтении журнала после следующей установки.
Завершение новой попытки не стирает предыдущую незакрытую.
