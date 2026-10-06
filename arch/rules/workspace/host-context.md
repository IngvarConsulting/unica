---
id: INV.WORKSPACE.HOST-CONTEXT
check:
  - crates/unica-coder/tests/host_workspace_context.rs::stdio_call_uses_host_workspace_instead_of_plugin_cwd
  - crates/unica-coder/tests/host_workspace_context.rs::two_frontends_and_interleaved_calls_keep_their_workspace_on_one_daemon
  - crates/unica-coder/tests/host_workspace_context.rs::startup_project_environment_and_request_override_are_forwarded
  - crates/unica-coder/tests/host_workspace_context.rs::required_or_malformed_context_never_falls_back_to_process_cwd
  - crates/unica-coder/tests/host_workspace_context.rs::conflicting_startup_project_environments_refuse_workspace_selection
  - crates/unica-coder/tests/host_workspace_context.rs::client_roots_override_stale_startup_environment_on_every_call
  - crates/unica-bootstrap/src/host/workspace_context.rs::first_client_root_outranks_stale_launch_environment_but_not_request_metadata
  - crates/unica-bootstrap/src/host/workspace_context.rs::a_supplied_root_satisfies_required_context_and_a_malformed_one_never_falls_back
  - crates/unica-coder/tests/host_workspace_context.rs::view_names_the_channel_that_chose_the_workspace_and_hints_only_for_stale_ones
  - crates/unica-coder/src/infrastructure/daemon/v13_workspace_bootstrap.rs::workspace_origin_is_named_next_to_every_workspace_root
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::workspace_origin_is_not_part_of_the_strict_submit_receipt_identity
---

# Рабочую папку вызова передаёт приложение-хост

Папка проекта или worktree определяется контекстом хоста вне аргументов
инструмента. Модель не выбирает её. Метаданные текущего вызова имеют приоритет
над переменными окружения, захваченными при запуске frontend. Контекст одного
вызова не изменяет папку следующих вызовов или других frontend общего демона.

Адаптер запрашивает у Codex `codex/sandbox-state-meta` и читает `sandboxCwd`
из `_meta` вызова: абсолютный путь либо локальный `file://` URI. Клиент,
объявивший MCP `roots` в сессии до протокола 2026-07-28, на каждый вызов без
таких метаданных отвечает на `roots/list`; рабочей папкой становится первый
root — локальный `file://` URI. Порядок roots спецификация MCP не задаёт:
первым Claude Code ставит каталог проекта сессии, а не каталог оболочки после
`cd`. Roots спрашиваются заново на каждый предметный вызов, поэтому новый
проект сессии подхватывается без перезапуска frontend; управление заданиями
рабочей папки не требует и roots не спрашивает. Пустой список или сорванный обмен —
ошибка, таймаут, закрытый транспорт — контекста не передают.

Переменные окружения захватываются при запуске frontend и стоят ниже roots:
Claude Code передаёт `CLAUDE_PROJECT_DIR`, ZCode — `ZCODE_PROJECT_DIR` или
совместимый `CLAUDE_PROJECT_DIR`. Если обе переменные заданы, они должны
указывать на одну папку. Расхождение roots с переменной ошибкой не считается.
Переданная хостом папка должна существовать.

Некорректный переданный контекст даёт отказ `invalid_state` без подстановки
другого проекта. Упакованный плагин требует контекст хоста; его технический
cwd не становится рабочим пространством. Прямой запуск бинарника без этого
требования сохраняет fallback на cwd запуска. Каталог демона остаётся
служебным, а рабочая папка передаётся в каждое предметное обращение явно.

Каждый ответ с `workspaceRoot` называет рядом в `workspaceRootOrigin` канал,
выбравший каталог (`requestMetadata`, `clientRoots`, `startupEnvironment`
с именем переменной, `launchCwd`), и сам запрошенный каталог: корень может
оказаться его предком. Если клиент в сессии до протокола 2026-07-28 объявил
roots, но канал запуска остался главным, ответ называет причину из закрытого набора (`empty`, `error`,
`timeout`, `closed`) без текста ошибки клиента. Для каналов, захваченных при
запуске, ответ подсказывает, как перейти к другому проекту. Канал объясняет
каталог и в идентичность запроса не входит.

Особенности приложений сосредоточены в [адаптере хоста](../platform/host-integration.md).
Разделение состояния сохраняет [идентичность workspace и профиля](source-profile-state-isolation.md).
