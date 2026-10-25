---
id: INV.WIRE.CHECK-DIAGNOSTIC-PAGES
check:
  - crates/unica-coder/src/application/v13/check.rs::check_pages_preserve_verdict_and_every_diagnostic_with_replay
  - crates/unica-coder/src/application/v13/check.rs::check_returns_whole_large_finding_and_refuses_late_oversize_before_page_one
  - crates/unica-coder/src/application/v13/check.rs::check_refuses_before_first_page_when_snapshot_cannot_be_retained
  - crates/unica-coder/src/application/v13/check.rs::late_finding_at_transport_edge_is_refused_before_issuing_a_cursor
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::bsl_check_never_calls_a_truncated_analyzer_result_complete
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::bsl_check_sees_an_error_after_two_hundred_warnings
  - crates/unica-coder/src/infrastructure/daemon/server.rs::root_check_pages_its_groups_by_count_and_bytes_with_the_whole_verdict
  - crates/unica-coder/src/infrastructure/daemon/server.rs::root_check_cursor_answers_only_the_question_that_issued_it
  - crates/unica-coder/src/infrastructure/daemon/server.rs::an_evicted_root_check_cursor_is_refused_instead_of_ending_the_walk
  - crates/unica-coder/tests/platform/receipt_runtime.rs::root_check_pages_are_the_same_through_a_task_and_a_direct_call
---

# Страницы диагностик сохраняют полный вердикт проверки

Адресный `check` вычисляет вердикт по всем находкам, затем выдаёт диагностики
страницами с неизменным вердиктом и точным повтором. Ошибка за пределами
первой страницы, в том числе после 200 предупреждений, участвует в вердикте.
Усечённый поставщиком анализ не объявляется полным.

До первой страницы проверяется возможность сохранить снимок и вернуть
каждую неделимую находку. У адресного `check` переполнение хранилища курсоров
или транспортного предела поздней находкой даёт отказ до обещания продолжения.
Большая находка передаётся целиком по [общему контракту страницы](read-page-budget.md).

Корневой `check {}` отдаёт вердикт, `checks` и `diagnosticCount` целиком на
каждой странице, а группы диагностик — страницами в порядке первого ответа.
Продолжение относится к тому же вердикту и не проверяет рабочее пространство
заново; курсор связан с рабочим пространством и `limit`. Курсор, чей вердикт
уже не хранится, получает `invalid_cursor`, а не завершённую страницу.
