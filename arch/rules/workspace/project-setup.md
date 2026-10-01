---
id: INV.SURFACE.WORKSPACE-BOOTSTRAP
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_does_not_hide_an_invalid_project_config
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_classifies_wrong_kind_project_config_as_invalid
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_classifies_broken_project_config_link_as_invalid
  - crates/unica-coder/src/infrastructure/daemon/v13_workspace_bootstrap.rs::project_config_recipe_quotes_yaml_significant_source_identity
  - crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_hands_the_project_file_recipe_without_an_initialize_operation
  - crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_names_mixed_source_formats_instead_of_recommending_a_project_file
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_repairs_a_valid_config_without_source_sets
---

# Корневой просмотр предлагает настройки и не записывает проектный файл

Когда проектного файла нет, `unica.view {}` сообщает `autodetected`
для найденных наборов и возвращает в `setup` путь `v8project.yaml`
с рекомендуемым содержимым. Для смешанных Designer и EDT наборов
единого содержимого нет: `setup.content` равно `null` с причиной.

Просмотр не создаёт файл и не предлагает снятую операцию
`workspace.initialize`; её вызов отвечает `unsupported_operation`.
Существующий проектный файл не перезаписывается: если в нём нет наборов,
ответ даёт пример настройки, сохраняя прежние байты файла.

Проверки рецепта и смешанных форматов вызывают собранный продукт по MCP.

Повреждённый проектный файл, каталог вместо файла и оборванная ссылка
дают `config.state: invalid` и отказ `invalid_state`. Автообнаружение
не скрывает эту ошибку. Проверка ссылки выполняется там, где её создание
поддерживается окружением.

Рекомендуемый YAML сохраняет имена и пути наборов, включая значимые
для YAML символы; это проверяется обратным разбором рецепта.
