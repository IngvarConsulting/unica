---
id: INV.APP.DOCUMENTATION-CONTAINER-FINGERPRINT
check:
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::a_replaced_container_is_reindexed_instead_of_answering_stale_help
  - crates/unica-coder/src/infrastructure/platform_help/provider.rs::a_second_call_answers_from_the_index_instead_of_rereading_the_installation
---

# Справка перечитывается при изменении отпечатка контейнера

Для контейнеров справки установленной платформы Unica сравнивает путь,
размер и время изменения файла. Если этот отпечаток изменился, следующий
запрос перестраивает индекс и получает страницы из обновлённого контейнера
без перезапуска процесса.

При прежнем отпечатке используется готовый индекс в памяти. Содержимое
файла для сравнения не перечитывается: замена байтов с сохранением размера
и времени изменения этим способом не обнаруживается.
