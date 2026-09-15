---
id: INV.WIRE.TERMINAL-RUN-HAS-NO-FENCE
status: active
governs: product
decision: DEC.2026-09-15.CLIENT-RUN-IS-TERMINAL-WITHOUT-A-FENCE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_client_run_binds_before_source_admission_without_a_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::preview_names_the_platform_without_dispatching_or_exposing_the_command
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::launch_reports_the_session_the_provider_attests
scope: [wire, product]
---

# Терминальная операция запускает без забора и не печатает команду

Операция словаря `unica.run` с `execution: terminal` исполняется по одному
вызову без `ifRev`: забор ревизии охраняет запись, а запуск клиента ничего
не записывает. `dryRun: true` у такой операции — необязательный план, и план
обязан прийти с `provider_dispatched: false`.

Ответ терминальной операции называет платформу версией и источником, сессию
— `pid`, ожидание — исходом; командную строку провайдера, пути установки и
журналов и вывод клиента опубликованная поверхность не печатает.
