---
id: DEC.2026-08-25.LOGICAL-READ-CORE-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/v13_read/tests.rs::logical_reader_parity_contract_is_complete
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
через существующие предметные readers и Task 13 projector. Неизвестный payload
reader-а отклоняется; физическое состояние и raw provider data не становятся
`props`. WebSocketClient остаётся логически допустимым, но source-backed чтение
отвечает `provider_unavailable`, пока versioned format spec и реальная export
fixture не докажут раскладку его модуля.

Continuation хранит уже нарезанные целые элементы или строки в ограниченном
process-local store. Непрозрачный cursor связан с адресом, выбранной адресом
проекцией, нормализованным filter, identity набора исходников, exact revision и
limit. Повтор cursor-а идемпотентен; смена revision даёт `stale_cursor`, другая
identity вопроса, подмена, неизвестность или expiry — `invalid_cursor`.

Внутренний `find` строится из actor-owned retained source roots и индексирует
только address/name/synonym/export-path/kind facts. BSL и body он не читает;
nearest ограничен name/address facts.

Широкие `DEC.2026-08-23.V0-13-EXECUTION-SURFACE` и
`DEC.2026-08-23.MODULE-CONTRACT` остаются `planned`. Production v0.12 и
`CTR.WIRE.TOOL-SURFACE` до Task 22 не меняются.

**Почему.** Tasks 15–21 должны опираться на одну форму чтения, один набор
typed-reader границ и одну политику продолжений до публичного cutover.

**Цена.** До Task 22 этот контракт crate-private; WebSocketClient source view
остаётся явным provider gap, а cursor не переживает перезапуск процесса.
