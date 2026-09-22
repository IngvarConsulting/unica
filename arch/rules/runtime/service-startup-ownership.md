---
id: INV.RUNTIME.SERVICE-STARTUP-OWNERSHIP
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_waits_for_peer_spawn_lock_release_before_reusing_record
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_spawn_lock_wait_observes_cancellation_and_shared_deadline
  - crates/unica-coder/src/infrastructure/workspace_services.rs::spawn_wait_ignores_live_record_not_owned_by_spawned_child
---

# Сервис не используется до завершения принадлежащего ему запуска

Пока другой клиент держит блокировку запуска, менеджер не использует даже
живую опубликованную запись сервиса. Он ждёт освобождения блокировки
в пределах исходного срока и с учётом отмены.

Проверка готовности нового процесса принимает только запись с PID и токеном
этого запуска; чужая живая запись её не заменяет. Проверки проходят настоящий
менеджер и блокировку с управляемыми соединением и запуском процесса.
