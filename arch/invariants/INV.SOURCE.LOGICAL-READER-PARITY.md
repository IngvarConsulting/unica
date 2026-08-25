---
id: INV.SOURCE.LOGICAL-READER-PARITY
status: active
governs: product
decision: DEC.2026-08-25.LOGICAL-READ-CORE-SLICE
check: crates/unica-coder/src/infrastructure/v13_read/tests.rs::logical_reader_parity_contract_is_complete
scope: [app, platform, product, source]
---

# Логический view делегирует предметным readers без универсального raw payload

Именованный non-skipping contract test строит реальный Platform XML source set
и проходит одиннадцать адресных случаев: configuration, metadata, form,
role/RLS, subsystem, interface facet, DCS, MXL, XDTO, common module и form
module binding. Reader-specific projections оставляют в `props` только
ограниченные локальные scalars, коллекции делают branches/items, а неизвестные
поля provider-а дают typed failure.

Bot использует доказанную зарегистрированную раскладку. WebSocketClient profile
видим в логическом дереве, но его source view остаётся явным
`provider_unavailable`, а не fake empty node или `not_found`.
