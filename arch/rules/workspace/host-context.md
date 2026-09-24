---
id: INV.WORKSPACE.HOST-CONTEXT
check:
  - crates/unica-coder/tests/host_workspace_context.rs::stdio_call_uses_host_workspace_instead_of_plugin_cwd
  - crates/unica-coder/tests/host_workspace_context.rs::two_frontends_and_interleaved_calls_keep_their_workspace_on_one_daemon
  - crates/unica-coder/tests/host_workspace_context.rs::startup_project_environment_and_request_override_are_forwarded
  - crates/unica-coder/tests/host_workspace_context.rs::required_or_malformed_context_never_falls_back_to_process_cwd
  - crates/unica-coder/tests/host_workspace_context.rs::conflicting_startup_project_environments_refuse_workspace_selection
---

# Рабочую папку вызова передаёт приложение-хост

Папка проекта или worktree определяется контекстом хоста вне аргументов
инструмента. Модель не выбирает её. Метаданные текущего вызова имеют приоритет
над переменными окружения, захваченными при запуске frontend. Контекст одного
вызова не изменяет папку следующих вызовов или других frontend общего демона.

Адаптер запрашивает у Codex `codex/sandbox-state-meta` и читает `sandboxCwd`
из `_meta` вызова: абсолютный путь либо локальный `file://` URI. Claude Code
передаёт `CLAUDE_PROJECT_DIR`, ZCode — `ZCODE_PROJECT_DIR` или совместимый
`CLAUDE_PROJECT_DIR`. Если обе переменные заданы, они должны указывать на одну
папку. Переданная хостом папка должна существовать.

Некорректный переданный контекст даёт отказ `invalid_state` без подстановки
другого проекта. Упакованный плагин требует контекст хоста; его технический
cwd не становится рабочим пространством. Прямой запуск бинарника без этого
требования сохраняет fallback на cwd запуска. Каталог демона остаётся
служебным, а рабочая папка передаётся в каждое предметное обращение явно.

Особенности приложений сосредоточены в [адаптере хоста](../platform/host-integration.md).
Разделение состояния сохраняет [идентичность workspace и профиля](source-profile-state-isolation.md).
