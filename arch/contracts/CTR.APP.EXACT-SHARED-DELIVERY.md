---
id: CTR.APP.EXACT-SHARED-DELIVERY
status: active
governs: product
decision: DEC.2026-08-24.EXACT-SHARED-DELIVERY-SLICE
check:
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::exact_delivery_progress_is_projected_to_the_owning_waiter
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::cancelling_one_delivery_follower_does_not_stop_the_process_owned_producer
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::pre_cancelled_delivery_returns_before_polling_and_never_publishes_progress
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::two_worktrees_join_one_identical_immutable_delivery
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::different_delivery_sha256_values_never_share
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::interrupted_archive_is_a_classified_failure_and_never_artifact_ready
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::delivery_boundary_rejects_non_delivery_key_mismatched_identity_and_relative_root
scope: [app]
version: 1
producer: crates/unica-coder/src/infrastructure/engine_delivery.rs
consumers: [review]
---

# Exact SharedWork поставки движка

Delivery key состоит ровно из artifact, version, target, SHA-256 и delivery
form. Producer может опубликовать только `ArtifactReady` с той же identity и
абсолютным install root либо `DeliveryFailure` закрытого класса. Одинаковый
неизменяемый ключ разделяется между worktree один раз, другой SHA-256 не
разделяется. Отмена одного V12 follower возвращает его compatibility state, но
не останавливает process-owned producer; progress наблюдает только владелец
ожидания. Классифицированный producer failure не становится ready.
