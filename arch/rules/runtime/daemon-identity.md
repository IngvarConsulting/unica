---
id: INV.APP.HIDDEN-SERVICES
check:
  - crates/unica-coder/src/infrastructure/daemon/identity.rs::production_identity_is_the_frozen_v5_digest_and_the_only_daemon_protocol
  - crates/unica-coder/src/infrastructure/daemon/identity.rs::every_canonical_core_identity_lives_under_the_protocol_v5_state_path
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::direct_runtime_entry_rejects_every_non_v5_identity_before_state_creation
  - crates/unica-coder/src/infrastructure/workspace_services.rs::hidden_service_identity_distinguishes_user_core_daemon_from_workspace_helpers
  - crates/unica-coder/tests/daemon_process.rs::two_frontend_processes_race_to_one_daemon_pid_record_and_endpoint
  - crates/unica-coder/src/infrastructure/daemon/identity.rs::executable_bytes_name_the_build_and_its_daemon_directory
  - crates/unica-coder/tests/host_workspace_context.rs::each_build_gets_its_own_daemon_and_a_replaced_build_is_refused
---

# Совместимые клиенты используют один демон в каталоге состояния пользователя

В пользовательском каталоге состояния демон определяется версией протокола
и идентичностью ядра. Рабочее пространство не входит в этот ключ.
Другое ядро получает отдельный каталог состояния.

Протокол между frontend и демоном внутренний: обратной совместимости он
не держит, меняется свободно и от выпускаемой версии не зависит. Поэтому
идентичность ядра включает сборку: у ядра, запущенного загрузчиком, — сумму
архива ядра для текущей цели из переданного им манифеста рантайма, иначе —
SHA-256 исполняемого файла. Frontend одной сборки не подключается к демону
другой. Если перед запуском демона сборка, из которой frontend его поднимет,
уже другая (заменён исполняемый файл или манифест), frontend отказывает
с требованием перезапустить хост.

Текущий бинарный файл запускает демон только протокола v5 со своей точной
идентичностью ядра. Идентичность снятого протокола v3 или другой сборки
отклоняется до создания файлов состояния.

Два одновременно запускающихся клиента одной идентичности подключаются
к одному PID и одному локальному адресу. Попытка запустить конкурирующий
демон не заменяет запись действующего владельца.
