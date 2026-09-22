---
id: INV.PERF.WORKSPACE-SERVICE-REUSE
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_reuses_matching_live_record_without_spawning
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_spawns_when_record_is_unreachable_or_version_mismatched
---

# Живой сервис рабочего пространства используется повторно

Менеджер использует уже запущенный сервис рабочего пространства, если запись
соответствует текущей версии и сервис отвечает на проверку доступности.
В этом случае новый процесс не запускается. Запись от другой версии
не используется: менеджер запускает новый сервис.

Проверки исполняют менеджер с управляемыми подключением и запуском процесса.
