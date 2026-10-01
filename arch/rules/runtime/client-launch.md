---
id: INV.WIRE.TERMINAL-RUN-HAS-NO-FENCE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_client_run_binds_before_source_admission_without_a_revision_gate
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_client_run_is_implemented_as_a_terminal_operation_with_closed_arguments
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::arguments_are_closed_and_each_refusal_names_the_fix
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::preview_names_the_platform_without_dispatching_or_exposing_the_command
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::launch_reports_the_session_the_provider_attests
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::waited_launch_reports_the_exit_code_and_the_timeout
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::a_preview_that_dispatched_a_client_is_refused_as_a_broken_contract
---

# Запуск клиента не требует предварительной ревизии

`run` с `op: "launch"` запускает клиент одним вызовом и доступен
без набора исходников. Операция имеет режим `terminal`: `ifRev`
отклоняется как `bad_value`. Необязательный `dryRun: true` возвращает план
без запуска; ответ раннера, сообщающий о запуске во время preview,
отклоняется.

Закрытые аргументы — `clientMode`, `execute`, `waitForExit`, `waitTimeoutMs`.
Ожидание внешней обработки требует одновременно `execute`, `waitForExit`
и положительного `waitTimeoutMs`. Ответ называет версию и источник
платформы, PID и исход ожидания. Команда раннера, пути установки и журналов,
вывод клиента и строка соединения в него не переносятся.

Сведения о сеансе подтверждает провайдер. Проверки исполняют адаптер
с управляемыми ответами раннера, без запуска настоящей платформы 1С.
