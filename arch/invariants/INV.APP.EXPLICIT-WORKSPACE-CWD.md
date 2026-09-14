---
id: INV.APP.EXPLICIT-WORKSPACE-CWD
status: active
governs: product
decision: DEC.2026-09-13.EXPLICIT-WORKSPACE-CWD
check:
  - crates/unica-coder/src/infrastructure/workspace.rs::absolute_requested_cwd_does_not_read_process_cwd
  - crates/unica-coder/src/infrastructure/workspace.rs::relative_requested_cwd_reports_launch_directory_failure
  - crates/unica-coder/src/infrastructure/workspace.rs::missing_requested_cwd_reports_launch_directory_failure
  - crates/unica-coder/src/infrastructure/workspace.rs::relative_requested_cwd_resolves_from_launch_directory
  - tests/ci/test_unica_mcp_smoke.py::test_workspace_hint_survives_deleted_frontend_and_daemon_cwd
scope: [app]
---

# Явный абсолютный cwd не читает каталог процесса

`discover_workspace` не обращается к каталогу процесса, если получил
абсолютный путь. Относительный путь разрешается от каталога процесса;
при его недоступности ошибка называет относительный выбор. Для отсутствующего
пути ошибка явно сообщает, что `cwd` не передан.

На ОС, допускающих удаление каталога работающего процесса, сохранённый
абсолютный workspace hint позволяет прочитать корень проекта и типизированные
метаданные через `unica.view`, а также найти BSL через `unica.search` после
удаления каталогов запуска frontend и daemon без перезапуска daemon.
