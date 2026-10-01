---
id: INV.PKG.STABLE-CATALOG-RELEASES-ONLY
check:
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_a_candidate_reaches_only_the_next_channel
  - tests/ci/test_release_channel.py::ChannelCatalogTests.test_a_candidate_never_reaches_the_stable_catalog
---

# Основной каталог называет только стабильный выпуск

Каталоги обоих хостов на ветке `main` маркетплейса указывают только на тег
стабильного выпуска `vX.Y.Z`. Потребители основного каталога не получают
кандидата `vX.Y.Z-rc.N` ни при установке, ни при обновлении.
