---
id: INV.RUNTIME.FULL-EXPORT-PLATFORM-TRUST
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::selected_executor_must_be_attested_locally_before_continue_or_stage_writes
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::full_export_rejects_invalid_results_before_replacing_sources
  - crates/unica-coder/tests/platform/full_dump_integration.rs::platform_resolver_does_not_trust_an_explicit_platform_path_or_broad_claim
  - crates/unica-coder/tests/platform/full_dump_integration.rs::platform_executable_or_install_swap_after_runner_prevents_publication
---

# Полная выгрузка запускает проверенную неизменяемую платформу

Перед исполнением и перед публикацией полной выгрузки проверяются
исполняемые файлы платформы, состав установки и сведения о её защищённости.
Непроверенная или изменившаяся установка не допускается к публикации.

В Windows требуется локальный том, неповышенный эффективный пользователь
и владелец установки из TrustedInstaller, LocalSystem либо Administrators.
Обычные пользователи могут читать и исполнять файлы, но не менять установку
или подменять её предков. Создание постороннего соседнего объекта выше
установки само по себе не считается её подменой. Неоднозначные ACL,
reparse points, несколько жёстких ссылок на файл и сетевые тома отклоняются.
