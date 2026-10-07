---
id: INV.PKG.NEXT-CHANNEL-NOT-OLDER
check:
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_a_release_reaches_both_channels_after_its_candidate
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_a_stable_release_opens_the_next_channel_when_none_exists
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_an_older_stable_release_leaves_the_newer_candidate_in_next
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_next_main_and_the_tag_move_in_one_push_and_a_rerun_completes_the_release
  - tests/ci/test_publish_channels.py::PublishChannelTests.test_a_candidate_overtaken_before_promote_leaves_next_alone
  - tests/ci/test_release_channel.py::ReleaseChannelTests.test_a_release_follows_its_own_prerelease
---

# Канал next не старше основного каталога

Каталоги обоих хостов на ветке `next` маркетплейса указывают на выпуск не
старше того, что называет основной каталог. Порядок — по SemVer: выпуск новее
своих кандидатов, поэтому `v0.13.0` сменяет в канале `v0.13.0-rc.3`.
В любой момент `next` не старше `main`: ветки каналов и тег выпуска уходят
одним атомарным push, и отказ любой из них не сдвигает ни одну.
`next` остаётся на месте, только если уже раздаёт более новый кандидат.
