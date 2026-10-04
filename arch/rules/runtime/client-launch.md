---
id: INV.WIRE.TERMINAL-RUN-HAS-NO-FENCE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_client_run_binds_before_source_admission_without_a_revision_gate
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_client_run_is_implemented_as_a_terminal_operation_with_closed_arguments
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::arguments_are_closed_and_each_refusal_names_the_fix
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::preview_names_the_platform_without_dispatching_or_exposing_the_command
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::launch_reports_the_session_the_provider_attests
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::unverified_launch_receipts_discard_owned_processes
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::cancellation_before_verified_handoff_discards_the_client
  - crates/unica-coder/src/infrastructure/platform/process.rs::pending_launch_release_preserves_detached_descendant
  - crates/unica-coder/src/infrastructure/platform/process.rs::pending_launch_rejection_drops_owned_descendant
  - crates/unica-coder/src/infrastructure/platform/process.rs::pending_launch_complete_json_without_eof_remains_cancelable
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::waited_launch_reports_the_exit_code_and_the_timeout
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::waited_launch_real_process_has_private_writable_logs_until_terminal_and_cleans_up
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::waiting_requires_a_thin_epf_but_nonwait_launch_modes_stay_available
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::waited_receipts_reject_inconsistent_identity_and_terminal_evidence
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::platform_receipts_keep_only_a_typed_version_or_explicit_unknown
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::timeout_words_without_a_wait_receipt_remain_an_ordinary_runner_failure
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::a_preview_that_dispatched_a_client_is_refused_as_a_broken_contract
---

# Запуск клиента выполняется одним вызовом

`run` с `op: "launch"` запускает клиент одним вызовом и доступен
без набора исходников. Операция имеет режим `terminal`; `dryRun` необязателен.
Как и остальные операции `run`, она не принимает `ifRev` и не выдаёт `rev`.
`dryRun: true` возвращает план
без запуска; ответ раннера, сообщающий о запуске во время preview,
отклоняется.

Закрытые аргументы — `clientMode`, `execute`, `waitForExit`, `waitTimeoutMs`.
Ожидание завершения поддерживается для `thin` с `execute` на `.epf`,
`waitForExit` и положительным `waitTimeoutMs`. Ответ называет версию и источник
платформы, PID и исход ожидания. Команда раннера, пути установки и журналов,
вывод клиента и строка соединения в него не переносятся.

При истечении `waitTimeoutMs` раннер завершает клиент. Подтверждённый
таймаут возвращает `provider_failed` с PID и исходом ожидания в `data`,
без записи в `changed` и предложения повторить запуск. Неуспешная
квитанция проходит общие проверки сеанса и ожидания; текст ошибки
провайдера не подтверждает таймаут.

Сведения о сеансе подтверждает провайдер. Проверки исполняют адаптер
с управляемыми ответами раннера, без запуска настоящей платформы 1С.

При запуске без ожидания Unica владеет деревом процессов до полного чтения
и проверки квитанции раннера. Отказ или отмена до передачи владения завершают
принадлежащие запуску процессы. Проверенная квитанция разрешает отделить
клиент от Unica; эта передача упорядочена с отменой. После неё отмена
не завершает подтверждённый сеанс и не заменяет результат запуска отказом.
Аварийное завершение между внешним эффектом и сохранением результата
обрабатывается по [правилу квитанций](invocation-at-most-once.md).
