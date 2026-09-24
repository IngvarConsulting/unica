---
id: INV.HOST.KNOWLEDGE-BEHIND-FACADE
check:
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_host_names_outside_the_host_facade
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_allows_host_names_only_in_the_host_facade_and_nested_host_tests
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_host_facade_root_is_not_granted_to_other_crates
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_repository_currently_complies_with_platform_boundary
gap: https://github.com/IngvarConsulting/unica/issues/983
---

# Особенности Codex, Claude и ZCode читает адаптер приложения-хоста

Чтение `CODEX_HOME`, `CLAUDE_PLUGIN_DATA`, `CLAUDE_PLUGIN_ROOT` и поиск
каталогов `.codex-plugin` и `.claude-plugin` сосредоточены в
`crates/unica-bootstrap/src/host/`. Общий код загрузчика получает
результат от этого адаптера.

Тот же адаптер читает каналы [рабочей папки](../workspace/host-context.md):
метаданные вызова Codex и переменные проекта Claude/ZCode. Протокольный
обработчик и демон используют его результат без ветвлений по имени хоста.

Проверки этих особенностей размещаются в
`crates/unica-bootstrap/tests/host/`. Такой же каталог в другом crate
не даёт его коду право обходить адаптер.

Различия хостов задаются дескрипторами внутри адаптера. Добавление хоста
не требует ветвлений по его имени в вызывающем коде: места вызова обходят
реестр дескрипторов. Текущие проверки границы не доказывают это расширение;
нужная проверка описана в `gap`.
