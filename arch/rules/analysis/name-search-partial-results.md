---
id: INV.SEARCH.NAMES-PARTIAL-RESULTS
check:
  - crates/unica-coder/src/infrastructure/v13_find.rs::unreadable_descriptor_makes_name_search_partial_but_resolve_refuses
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_local_io_error_is_partial_but_handle_failure_is_a_refusal
  - crates/unica-coder/src/infrastructure/v13_find.rs::cancellation_during_a_failed_descriptor_read_refuses_instead_of_returning_partial
  - crates/unica-coder/src/infrastructure/v13_find.rs::descriptor_identity_drift_remains_a_refusal
  - crates/unica-coder/src/infrastructure/v13_find.rs::linked_nested_descriptor_refuses_instead_of_looking_complete
  - crates/unica-coder/src/infrastructure/v13_find.rs::linked_proven_owner_directory_refuses_instead_of_hiding_nested_names
  - crates/unica-coder/src/infrastructure/v13_find.rs::linked_command_directory_refuses_instead_of_looking_complete
  - crates/unica-coder/src/infrastructure/v13_find.rs::many_owner_directories_do_not_exhaust_open_file_handles
  - crates/unica-coder/src/infrastructure/daemon/mod.rs::name_search_reports_an_injected_local_read_fault_through_the_live_daemon
  - crates/unica-coder/tests/platform/v13_search_unreadable.rs::unreadable_name_descriptor_is_reported_without_losing_proven_matches
---

# Ошибка чтения одного объекта делает поиск имён явно неполным

При локальной ошибке чтения конкретного объекта `unica.search` по именам
возвращает остальные доказанные совпадения и сообщает о неполноте результата,
причине пропуска и проблемном объекте, если его идентичность установлена.
Непроверенные сведения не становятся достоверными совпадениями.
Поле `sourceCoverage` показывает, удалось ли прочитать кандидаты в дескрипторы:
`complete: false` и `omitted` отмечают пропуски. Диагностика ограничена и не
выдаёт физические пути или логический адрес, полученный только из имени файла.

Пустой неполный результат не доказывает, что искомого объекта нет.
Признак `approximate` сообщает о близости имени, а не о полноте поиска.

Отмена, истечение срока и потеря границы или согласованности исходников
остаются отказами. Правило не меняет контракт точного `resolve`
и не разрешает исправлять повреждённые исходники в ходе поиска.
