---
id: INV.APP.WORKSPACE-SERVICE-STARTUP-DIAGNOSTIC
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::startup_failure_reports_exit_status_and_bounded_stderr_tail
  - crates/unica-coder/src/infrastructure/workspace_services.rs::startup_stderr_tail_stays_bounded_after_lossy_utf8_decoding
  - crates/unica-coder/src/infrastructure/workspace_services.rs::startup_failure_reports_live_child_without_assuming_deadline_expired
---

# Ошибка запуска сервиса различает завершение процесса и отсутствие готовности

Если процесс сервиса завершился до готовности, ошибка сохраняет исходную
причину ожидания и добавляет код выхода с ограниченным хвостом stderr.
Начало большого журнала не вытесняет его конец. Некорректный UTF-8
не увеличивает разрешённый размер сообщения.

Если процесс ещё жив, сообщение называет это состояние. Отмена ожидания
не объявляется истечением срока. Проверки запускают настоящий дочерний
процесс и исполняют сборку диагностического сообщения.
