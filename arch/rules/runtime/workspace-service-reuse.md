---
id: INV.PERF.WORKSPACE-SERVICE-REUSE
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_reuses_matching_live_record_without_spawning
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_spawns_when_record_is_unreachable_or_version_mismatched
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_replaces_a_service_of_another_build_and_shuts_it_down
  - crates/unica-coder/src/infrastructure/workspace_services.rs::service_record_is_reusable_only_for_matching_live_build_and_paths
  - crates/unica-coder/src/infrastructure/workspace_services.rs::a_schema_four_binary_can_still_read_and_reject_the_record
---

# Живой сервис рабочего пространства используется повторно

Менеджер использует уже запущенный сервис рабочего пространства, если запись
называет текущую сборку ядра и сервис отвечает на проверку доступности.
В этом случае новый процесс не запускается. Протокол между демоном и сервисом
внутренний, как и [протокол демона](daemon-identity.md), поэтому сборка
сверяется по идентичности ядра, а не по версии пакета.

На один корень исходников проекта приходится один сервис: индексы анализатора
лежат в общем кеше проекта. Сервис другой сборки или записи прежней схемы
не используется: менеджер просит его завершиться и запускает свой. Поэтому
два хоста разных сборок, работающие над одним проектом одновременно,
поочерёдно перезапускают сервис.

Запись сохраняет поля, обязательные для бинарей прежней схемы. Так и они
могут погасить сервис новой сборки, а не оставить его жить рядом со своим.

Проверки исполняют менеджер с управляемыми подключением и запуском процесса.
