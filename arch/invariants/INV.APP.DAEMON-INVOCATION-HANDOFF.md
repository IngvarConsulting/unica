---
id: INV.APP.DAEMON-INVOCATION-HANDOFF
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/application/invocation.rs::canonical_handoff_boundary_is_direct_before_7000_and_durable_at_or_before_deadline
scope: [app]
---

# Седьмая секунда разделяет direct response и durable Task

Завершение до 7000 мс может вернуть direct DomainResult. В 7000 мс
незавершённая Invocation уже имеет durable Task; нулевой бюджет материализует её
до execution. Конкурентные completion и handoff публикуют один terminal result
при одном execution.
