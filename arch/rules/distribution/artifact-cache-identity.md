---
id: INV.PKG.ARTIFACT-CACHE-IDENTITY
check:
  - crates/unica-coder/src/infrastructure/bundled_tools.rs::tools_sharing_one_archive_name_one_artifact
  - crates/unica-bootstrap/tests/runtime_install.rs::the_install_path_is_keyed_by_version_and_archive_identity
  - crates/unica-bootstrap/tests/runtime_install.rs::rebuilt_artifact_with_the_same_version_gets_a_distinct_immutable_root
  - crates/unica-bootstrap/tests/runtime_install.rs::a_plugin_release_does_not_refetch_an_unchanged_artifact
  - crates/unica-bootstrap/tests/runtime_install.rs::an_engine_is_installed_on_demand_under_its_own_version
---

# Обновление плагина использует неизменённые компоненты из кеша

Артефакт — отдельно доставляемый компонент, например ядро Unica или движок
поиска. Его каталог внутри кеша имеет вид
`<артефакт>/<его версия>--<SHA-256 поставки>/<целевая платформа>`.

Запрос инструмента разрешается в имя его артефакта. Несколько инструментов
из одного артефакта, например `rlm-bsl-index` и `rlm-bsl-mcp`, используют
одну установку в кеше.

Версия плагина в ключ не входит. Если готовая установка компонента сохранилась
и проходит проверку, обновление плагина использует её без повторной загрузки.

Другая сборка того же компонента и версии с иной SHA-256 получает отдельный
каталог. Публикация новой сборки не перезаписывает файлы прежней.
