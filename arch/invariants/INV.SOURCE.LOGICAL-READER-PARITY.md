---
id: INV.SOURCE.LOGICAL-READER-PARITY
status: active
governs: product
decision: DEC.2026-08-25.LOGICAL-READ-CORE-SLICE
check: crates/unica-coder/src/infrastructure/v13_read/tests.rs::logical_reader_parity_contract_is_complete
scope: [app, platform, product, source]
---

# Логический view делегирует предметным readers без универсального raw payload

Именованный non-skipping contract test строит реальные Platform XML source sets,
проходит все 19 `validAddresses` профиля и одиннадцать retained reader cases:
configuration, metadata, form, role/RLS, subsystem, interface facet, DCS, MXL,
XDTO, common module и form module binding. Branch count равен длине достижимой
collection, каждый projector потребляет весь suffix, а remap source set после
admission не меняет bytes или revision authority. Reader-specific projections
оставляют в `props` только ограниченные локальные scalars, коллекции делают
branches/items, неизвестные поля provider-а дают typed failure, а неподдержанный
filter — `bad_value`.

Зарегистрированный owner имеет ровно одну profile-derived `Module` branch:
branch count равен числу уникальных допустимых ролей, а все 25 положительных
`moduleCapabilities` профиля покрыты production retained authorities для
configuration, EPF и ERF через parent navigation и find, включая
зарегистрированный owner без Module.bsl. Отсутствующий в inventory owner не
получает Module branch. Внешние source sets доказывают каждый top-level artifact
строгим descriptor-ом, не публикуют configuration runtime modules и имеют
bounded/cancellable aggregate inventory read. Role объединяет allowed/denied по
canonical `(kind, name)`, размещает уникальные RLS nodes только под Right и
отклоняет неоднозначный короткий alias с canonical кандидатами.
Отсутствующий HomePage sidecar допустим, present malformed/wrong-root evidence
даёт `provider_unavailable`; V12 legacy wrapper сохраняет старую трактовку.
Общий 120-секундный operation budget view/find отделён от 7-секундного Task
handoff и использует cancellation.

Bot использует доказанную зарегистрированную раскладку. WebSocketClient profile
видим в логическом дереве, но его source view остаётся явным
`provider_unavailable`, а не fake empty node или `not_found`.
