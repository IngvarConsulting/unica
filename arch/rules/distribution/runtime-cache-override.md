---
id: INV.CACHE.OVERRIDE-PRIORITY
check:
  - crates/unica-bootstrap/src/host/runtime_cache.rs::an_unexpanded_override_falls_through_to_the_host_chain
  - crates/unica-bootstrap/src/host/runtime_cache.rs::a_published_data_directory_outranks_every_home_directory
  - crates/unica-bootstrap/src/host/runtime_cache.rs::a_declared_host_home_is_used_as_it_stands
  - crates/unica-bootstrap/src/host/runtime_cache.rs::the_user_home_carries_the_host_state_directory
  - crates/unica-bootstrap/src/host/runtime_cache.rs::the_windows_user_home_is_the_last_resort
  - crates/unica-bootstrap/src/host/runtime_cache.rs::the_posix_user_home_wins_over_the_windows_one
  - crates/unica-bootstrap/src/host/runtime_cache.rs::an_empty_environment_names_every_variable_that_would_help
  - crates/unica-bootstrap/src/host/runtime_cache.rs::an_unexpanded_override_alone_is_not_a_cache_root
  - crates/unica-bootstrap/src/host/runtime_cache.rs::the_explicit_override_outranks_every_host_source
---

# Явно заданный каталог установки важнее настроек хоста

Если `UNICA_RUNTIME_CACHE_DIR` содержит готовый путь, установщик Unica
выбирает его как корень каталога исполняемых компонентов (runtime).
Настройки `CLAUDE_PLUGIN_DATA`, `CODEX_HOME` и домашнего каталога пользователя
не заменяют этот путь и не добавляют к нему подкаталог.

Если готового пути нет, каталог выбирается в порядке:
`<CLAUDE_PLUGIN_DATA>/runtimes`, `<CODEX_HOME>/unica/runtimes`,
`<HOME>/.codex/unica/runtimes`, затем `<USERPROFILE>/.codex/unica/runtimes`.
Значение override с нераскрытым `${` пропускается: каталог с буквальным
шаблоном не создаётся. Без пригодного значения из этой цепочки установка
отказывает.
