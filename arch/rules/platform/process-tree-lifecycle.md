---
id: INV.PLATFORM.PROCESS-TREE-LIFECYCLE
check:
  - crates/unica-process/src/platform/process.rs::managed_process_tree_lifecycle_is_bounded
  - crates/unica-bootstrap/src/platform/process.rs::job::closing_the_bootstrap_job_terminates_the_runtime
  - crates/unica-bootstrap/src/platform/process.rs::job::a_failed_assignment_terminates_the_suspended_runtime
---

# Остановка управляемого запуска охватывает его дочерние процессы

Отмена и освобождение управляемого запуска, а также истечение срока,
если его задал пользователь, завершают процесс и принадлежащее ему дерево
потомков. Срока по умолчанию у запуска нет. Явная очистка процесса при запуске
сервиса действует так же. Ожидание завершения ограничено по времени.

На Windows дочерний процесс начинает работу только после включения в группу,
которой владеет Unica (`Job Object`). Правило не относится к запуску,
который был явно отделён от владельца через `detach`.
