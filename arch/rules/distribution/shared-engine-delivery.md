---
id: CTR.APP.EXACT-SHARED-DELIVERY
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_shared_delivery_releases_request_admission_before_wait_and_shares_across_worktrees
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::exact_delivery_progress_is_projected_to_the_owning_waiter
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::cancelling_one_delivery_follower_does_not_stop_the_process_owned_producer
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::pre_cancelled_delivery_returns_before_polling_and_never_publishes_progress
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::two_worktrees_join_one_identical_immutable_delivery
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::different_delivery_sha256_values_never_share
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::interrupted_archive_is_a_classified_failure_and_never_artifact_ready
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::delivery_boundary_rejects_non_delivery_key_mismatched_identity_and_relative_root
---

# Потребители одной поставки движка используют общую доставку

В одном `DeliveryDesk` доставка определяется артефактом, версией, целевой
платформой, SHA-256 и формой поставки. Потребители с одинаковым ключом
используют одну доставку, даже если работают с разными рабочими каталогами.
Изменение любого поля означает другую доставку.

Акторы одного демона используют общий координатор доставки. Вызовы получают
фоновые задания до ожидания исполнителя, поэтому ожидание не удерживает
допуск нового запроса. Это проверяется на границе демона с внедрённым
сервисом доставки.

Успех содержит тот же ключ и абсолютный путь установки. Ошибка имеет
определённый класс и не превращается в готовую поставку. Например,
прерванный архив нельзя объявить установленным.

Доставка принадлежит процессу: отмена одного ожидающего вызова не останавливает
исполнителя. Ожидающий вызов получает прогресс своего артефакта — полученные
байты и общий размер, если он известен. Вызов, отменённый до ожидания,
возвращается сразу и не публикует прогресс.
