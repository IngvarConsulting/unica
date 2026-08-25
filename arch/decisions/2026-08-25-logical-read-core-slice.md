---
id: DEC.2026-08-25.LOGICAL-READ-CORE-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/mod.rs::injected_hidden_v13_service_executes_real_view_and_find_through_actor_capabilities
supersedes: []
superseded-by: null
establishes: [CTR.SOURCE.LOGICAL-NODE-VIEW-SHAPE, INV.SOURCE.LOGICAL-READER-PARITY, INV.SOURCE.REVISION-BOUND-VIEW-CURSOR, INV.SOURCE.FIND-IDENTITY-ONLY]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Скрытое ядро v0.13 читает логическое дерево единым контрактом

**Решение.** Внутренний `view` выбирает типизированную проекцию только
квалифицированным логическим адресом. Семь общих слотов узла не смешиваются с
конвертом результата, а `items` принадлежат только адресуемой ветке. Данные
строк и исходные строки адреса не получают.

Metadata, form, role/RLS, subsystem/interface, DCS, MXL, XDTO и module читаются
через один actor-supplied retained read authority, существующие предметные
parsers и Task 13 projector. Каждый reader получает ту же exact revision
authority и не переопределяет source set через изменившийся workspace config.
Неизвестный payload reader-а отклоняется; физическое состояние и raw provider
data не становятся `props`. WebSocketClient остаётся логически допустимым, но
source-backed чтение отвечает `provider_unavailable`, пока versioned format
spec и реальная export fixture не докажут раскладку его модуля.

Continuation хранит уже нарезанные целые элементы или строки в ограниченном
process-local store. Непрозрачный cursor связан с адресом, выбранной адресом
проекцией, нормализованным filter, identity набора исходников, exact revision и
limit. Повтор cursor-а идемпотентен; смена revision даёт `stale_cursor`, другая
identity вопроса, подмена, неизвестность или expiry — `invalid_cursor`.

Внутренний `find` проходит то же адресуемое дерево из actor-owned retained
source roots и индексирует только address/name/synonym/export-path/kind facts.
Для module/method/region identities он ограниченно разбирает декларативную
оболочку BSL, но не индексирует `Body`, statements или строки исходника и не
обходит Body-проекции повторно. Одинаковые declarations/regions при разных
statements дают одинаковые facts и результаты; nearest ограничен name/address
facts. Зарегистрированный owner публикует одну profile-derived `Module` branch со
всеми допустимыми ролями даже без module source; отсутствующий owner не
изобретается. WebSocketClient сохраняется из parent projection, а прямой view — gap.

Один logical-read operation budget в 120 секунд ограничивает и `view`, и
aggregate `find`, передаётся actor-owned revision/provider границе и проверяет
cancellation. Это не семисекундный handoff: после handoff тот же callback один
раз продолжает работу и публикует terminal Task. Injected service исполняет оба
handler-а на actor capability; production default dormant до Task 22. Present
malformed/wrong-root HomePage отклоняется; role rights сливаются в canonical node.

Широкие `DEC.2026-08-23.V0-13-EXECUTION-SURFACE` и
`DEC.2026-08-23.MODULE-CONTRACT` остаются `planned`. Production v0.12 и
`CTR.WIRE.TOOL-SURFACE` до Task 22 не меняются.

**Почему.** Tasks 15–21 должны опираться на одну форму чтения, typed-reader
границы и одну политику продолжений до публичного cutover.

**Цена.** До Task 22 контракт crate-private; WebSocketClient source view — gap,
find читает bounded BSL declaration shell, cursor не переживает перезапуск.
