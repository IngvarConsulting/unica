---
id: INV.RUNTIME.STORED-TASK-SECRETS
check:
  - crates/unica-coder/src/infrastructure/daemon/server/stored_secret_tests.rs::a_connection_secret_from_configuration_and_runner_answer_is_not_stored_or_read_back
---

# Сохранённое состояние задания не раскрывает секреты подключения

Данные аутентификации и другие секреты подключения не попадают в сохранённые
вызовы, задачи, сообщения состояния и устойчивые журналы. Ответ поставщика
не освобождает Unica от этого ограничения.
