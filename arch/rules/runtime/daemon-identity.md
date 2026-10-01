---
id: INV.APP.HIDDEN-SERVICES
check:
  - crates/unica-coder/src/infrastructure/daemon/identity.rs::production_identity_is_the_frozen_v5_digest_and_the_only_daemon_protocol
  - crates/unica-coder/src/infrastructure/daemon/identity.rs::every_canonical_core_identity_lives_under_the_protocol_v5_state_path
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::direct_runtime_entry_rejects_every_non_v5_identity_before_state_creation
  - crates/unica-coder/src/infrastructure/workspace_services.rs::hidden_service_identity_distinguishes_user_core_daemon_from_workspace_helpers
  - crates/unica-coder/tests/daemon_process.rs::two_frontend_processes_race_to_one_daemon_pid_record_and_endpoint
---

# Совместимые клиенты используют один демон в каталоге состояния пользователя

В пользовательском каталоге состояния демон определяется версией протокола
и идентичностью ядра. Рабочее пространство не входит в этот ключ.
Другое ядро получает отдельный каталог состояния.

Текущий бинарный файл запускает демон только протокола v5 со своей точной
идентичностью ядра. Идентичность снятого протокола v3 или другого ядра
отклоняется до создания файлов состояния.

Два одновременно запускающихся клиента одной идентичности подключаются
к одному PID и одному локальному адресу. Попытка запустить конкурирующий
демон не заменяет запись действующего владельца.
