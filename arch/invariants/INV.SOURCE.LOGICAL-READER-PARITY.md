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

Exact revision строится тем же retained capability, что и bytes. Один
operation-scoped lease захватывает выбранный source set для qualified `view` и
все admitted source sets для aggregate `find`; unrelated sibling не сканируется
и не инвалидирует `view`. Retained manifest помечен physical identity и только
matching retained provenance может удовлетворить retained fast path; ambient и
retained provenance симметрично не взаимозаменяемы. При unsupported fence
initial capture и final confirmation состоят из двух descriptor-relative passes
каждый: post-order named identity и directory membership должны сохраниться, а
semantic manifest и отдельная physical identity evidence — совпасть. Stable
operation даёт четыре passes; три bounded attempts ограничивают один capture
шестью, всю operation — двенадцатью passes, а node reads corpus не сканируют.
Semantic digest остаётся byte-compatible с ambient algorithm. Каждый pass
потоковый, проверяет cancellation между chunks и ограничивает entries, file
bytes и aggregate bytes.
Любой contributing read, canonical Role resolution и owner proof предшествует
последнему exact-revision fence, поэтому replacement/mutation даёт coherent
retained result либо typed stale/invalidation, но не mixed revision; после
final confirmation source I/O нет.

До dispatch всех typed readers действует один recursive owner admission:
top-level `(kind,name)` присутствует в inventory и имеет matching descriptor, а
каждый физический Form/Template/Command зарегистрирован parent `ChildObjects` и
имеет matching child descriptor. Orphan physical content, registered missing
descriptor и wrong kind/name fail closed одинаково в direct view, parent
navigation и find; evidence и edges кешируются только внутри actor/revision.

Зарегистрированный owner имеет ровно одну profile-derived `Module` branch:
branch count равен числу уникальных допустимых ролей, а все 25 положительных
`moduleCapabilities` профиля покрыты production retained authorities для
configuration, EPF и ERF через parent navigation и find, включая
зарегистрированный owner без Module.bsl. Отсутствующий в inventory owner не
получает Module branch. Внешние source sets доказывают каждый top-level artifact
строгим descriptor-ом, не публикуют configuration runtime modules и имеют
bounded/cancellable aggregate inventory read. Role объединяет allowed/denied по
canonical `(kind, name)`, размещает уникальные RLS nodes только под Right и
отклоняет неоднозначный короткий alias с canonical кандидатами. V13 принимает
в Role только kinds из точного platform registry `METADATA_KIND_TAGS` без `_`,
поэтому canonical `kind_name`
инъективен; произвольный type prefix даёт `provider_unavailable`, не duplicate
`at`.
Отсутствующий HomePage sidecar допустим, present malformed/wrong-root evidence
даёт `provider_unavailable`; V12 legacy wrapper сохраняет старую трактовку.
Общий абсолютный 120-секундный operation budget view/find без replenishment
отделён от 7-секундного Task handoff и использует cancellation на admission,
reader и final confirmation. Terminal actor publication и rejected-address
classification принадлежат `INV.SOURCE.RETAINED-LOGICAL-PUBLICATION`.

Bot использует доказанную зарегистрированную раскладку. WebSocketClient profile
видим в логическом дереве, но его source view остаётся явным
`provider_unavailable`, а не fake empty node или `not_found`.
