---
id: INV.PKG.MARKETPLACE-TAG-IS-PUBLISHED-CATALOG
check:
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_the_release_tag_is_the_published_catalog
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_a_stable_release_kept_out_of_next_is_tagged_on_main
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_next_main_and_the_tag_move_in_one_push_and_a_rerun_completes_the_release
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_install_checks_resolve_the_candidate_anchor
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_a_rerun_of_a_completed_publication_changes_nothing
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_a_release_tag_with_other_bytes_stops_the_publication
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_an_anchor_with_other_bytes_stops_the_install_checks
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_a_tag_left_on_the_staging_commit_is_kept_and_the_catalog_completes
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_a_push_refused_once_is_rebuilt_and_published
  - tests/ci/test_release_channel.py::ChannelCatalogTests.test_catalogs_resolve_only_the_release_or_its_anchor
---

# Тег выпуска в маркетплейсе — снимок опубликованного каталога

Тег `vX` в `IngvarConsulting/unica-marketplace` называет коммит, каталоги
обоих хостов в котором указывают на `vX`, а `plugins/unica` совпадает
с деревом, прошедшим проверки установки. Поэтому установка с `--ref vX`
даёт ровно версию X. Тег стабильного выпуска стоит на коммите продвижения
ветки `main`, тег кандидата — на коммите продвижения ветки `next`.

Тег появляется в одном атомарном push с ветками каналов, которые выпуск
сдвигает: тега нет без каталога, который его называет, и каталог не называет
отсутствующий тег. Проверки установки до продвижения разрешают якорь
`candidate-vX` на коммите stage. Опубликованные `vX` и `candidate-vX`
не двигаются: повтор публикации сверяет дерево `plugins/unica` найденного тега
с проверенным и при расхождении останавливается.

Теги с `v0.9.1` по `v0.13.0-rc.5` поставлены на коммит stage и называют
каталог предыдущей версии; это их неизменяемое состояние, а не нарушение.
