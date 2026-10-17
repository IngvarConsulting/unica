---
id: INV.APP.DOCUMENTATION-CONTAINER-FINGERPRINT
check:
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::a_replaced_container_is_reindexed_instead_of_answering_stale_help
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::a_second_call_answers_from_the_index_instead_of_rereading_the_installation
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::a_second_language_on_the_same_installation_does_not_answer_from_the_first_ones_index
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::two_requests_resolving_to_the_same_locale_share_one_index
---

# Индекс справки переиспользуется без смешения корпусов

Индекс справки живёт в памяти. Корпуса разных установок и языков не смешиваются;
запросы, выбравшие одни и те же контейнеры, используют готовый индекс.

Отпечаток контейнера включает путь, размер и время изменения. При изменении
отпечатка следующий запрос перестраивает индекс без перезапуска процесса.
Замена байтов с сохранением размера и времени изменения так не обнаруживается.

Границы копирования корпуса и записи на диск задаёт
[правило материалов справки](../../distribution/platform-documentation-corpus.md).
