---
id: INV.APP.DOCUMENTATION-CONTAINER-FINGERPRINT
check:
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::a_replaced_container_is_reindexed_instead_of_answering_stale_help
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::a_second_call_answers_from_the_index_instead_of_rereading_the_installation
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::a_second_language_on_the_same_installation_does_not_answer_from_the_first_ones_index
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::two_requests_resolving_to_the_same_locale_share_one_index
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_plugin_source_copy_excludes_vendor_corpus_outside_the_plugin_root
gap: https://github.com/IngvarConsulting/unica/issues/953
---

# Индекс справки переиспользуется без смешения корпусов и копий на диске

Индекс справки живёт в памяти. Корпуса разных установок и языков не смешиваются;
запросы, выбравшие одни и те же контейнеры, используют готовый индекс.

Отпечаток контейнера включает путь, размер и время изменения. При изменении
отпечатка следующий запрос перестраивает индекс без перезапуска процесса.
Замена байтов с сохранением размера и времени изменения так не обнаруживается.

Разбор справки не записывает контейнеры, индекс и распакованные страницы
на диск. Материалы вендора не копируются в репозиторий и не входят в пакет.

Проверка упаковки подтверждает исключение корпуса вне корня плагина.
Отсутствие записей при работе поставщика пока не закреплено отдельной проверкой.
