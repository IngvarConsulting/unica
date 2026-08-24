---
id: CTR.APP.EXACT-SHARED-DELIVERY
status: active
governs: product
decision: DEC.2026-08-24.EXACT-SHARED-DELIVERY-SLICE
check: crates/unica-coder/src/infrastructure/engine_delivery.rs::exact_delivery_identity_failure_and_follower_cancellation_are_one_contract
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
