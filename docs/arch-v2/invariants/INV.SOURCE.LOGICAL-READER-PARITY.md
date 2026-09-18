---
id: INV.SOURCE.LOGICAL-READER-PARITY
---


# Логический view делегирует предметным readers без универсального raw payload

Именованный non-skipping contract test строит реальные Platform XML source sets,
проходит все 19 `validAddresses` профиля и одиннадцать retained reader cases:
configuration, metadata, form, role/RLS, subsystem, interface facet, DCS, MXL,
XDTO, common module и form module binding. Branch count равен длине достижимой
collection, каждый projector потребляет весь suffix. Reader-specific projections
оставляют в `props` только ограниченные локальные scalars, коллекции делают
branches/items, неизвестные поля provider-а дают typed failure, а неподдержанный
filter — `bad_value`.

Чтение каждого логического узла не повторяет полный обход исходников.
После последнего exact-revision fence source I/O нет; это ограниченная
оптимистическая стабилизация, без гарантии против синхронизированного ABA writer.

Command зарегистрирован полным инлайн-определением
`<Command uuid><Properties><Name>` в parent `ChildObjects`.
Evidence и edges кешируются только внутри actor/revision.

Зарегистрированный owner имеет ровно одну profile-derived `Module` branch:
branch count равен числу уникальных допустимых ролей, а все 25 положительных
`moduleCapabilities` профиля покрыты production retained authorities для
configuration, EPF и ERF через parent navigation, включая
зарегистрированный owner без Module.bsl. Отсутствующий в inventory owner не
получает Module branch. Внешние source sets доказывают каждый top-level artifact
строгим descriptor-ом, не публикуют configuration runtime modules и имеют
bounded/cancellable aggregate inventory read. Bot использует доказанную зарегистрированную раскладку. WebSocketClient profile
видим в логическом дереве, но его source view остаётся явным
`provider_unavailable`, а не fake empty node или `not_found`.
