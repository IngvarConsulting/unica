---
id: INV.SAFETY.DAEMON-ENDPOINT-OWNERSHIP
check:
  - crates/unica-daemon/src/client_v5.rs::failed_startup_cleanup_never_adopts_successor_with_reused_pid
  - crates/unica-coder/tests/daemon_process/endpoint_ownership.rs::idle_daemon_removes_its_own_endpoint_after_actual_process_exit
  - crates/unica-coder/tests/daemon_process/endpoint_ownership.rs::idle_daemon_preserves_same_pid_successor_on_the_original_file
  - crates/unica-coder/tests/daemon_process/endpoint_ownership.rs::idle_daemon_preserves_a_byte_identical_replacement_file
  - crates/unica-coder/tests/daemon_process/endpoint_ownership.rs::frontend_refuses_an_insecure_daemon_endpoint_file
  - crates/unica-coder/tests/daemon_process/endpoint_ownership.rs::frontend_refuses_an_insecure_daemon_endpoint_directory
  - crates/unica-coder/tests/platform/daemon_endpoint_security.rs::frontend_refuses_an_insecure_acl_even_with_private_posix_mode
  - crates/unica-coder/tests/platform/daemon_endpoint_security.rs::frontend_refuses_an_insecure_directory_acl_even_with_private_posix_mode
  - crates/unica-workspace/src/platform/filesystem/macos_acl.rs::inherited_foreign_allow_refuses_private_children_before_returning_a_writable_file
  - crates/unica-workspace/src/platform/filesystem/macos_acl.rs::private_acl_accepts_owner_allow_and_non_owner_deny
---

# Запись подключения демона доступна владельцу и удаляется только им

Каталог и файл подключения демона доступны только владельцу. Для подключения
создаётся случайный токен. Завершающийся демон удаляет только свою запись:
замена её записью другого владельца, даже с тем же PID, не даёт права удаления.

Проверки подтверждают права каталога и файла средствами ОС и отказ подключения
к небезопасной записи. После фактического завершения настоящего демона
проверяется удаление его записи, сохранение записи преемника с тем же PID
и сохранение нового физического файла с прежним содержимым. Проверка очистки
неудачного запуска сохраняется. Случайность токена определяется способом его
создания, а не статистическим тестом.
