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

**Решение.** Внутренний `view` выбирает typed projection только qualified address. Семь слотов узла отделены от result envelope; `items` принадлежат адресуемой ветке, а data/source rows адреса не получают.

Все readers используют одну actor-supplied retained authority, общие предметные parsers, Task 13 projector и exact revision. Raw payload отклоняется. Configuration/extension inventory доказывается `Configuration.xml`, EPF/ERF — строгими top-level artifact descriptors; `ConfigDumpInfo.xml`, malformed/ambiguous evidence, configuration runtime modules и фиктивный configuration export path во внешнем source set не принимаются. Module допустим только у доказанного owner-а, но отсутствие BSL не мешает будущему create. WebSocketClient остаётся profile identity с прямым `provider_unavailable` до доказанной раскладки.

Bounded process-local cursor связан с address-selected projection, normalized filter, source identity, exact revision и limit. Retry идемпотентен; revision change даёт `stale_cursor`, а чужая identity, tamper, unknown или expiry — `invalid_cursor`.

`find` обходит то же retained tree и индексирует только address/name/synonym/export-path/kind. Module/method/region берутся из bounded BSL declaration shell; `Body`, statements и source lines не индексируются. Owner inventory и module projection вычисляются один раз на `source identity + exact revision + canonical address` и не переходят между actor/revision.

Единый 120-секундный logical-read budget для `view`/aggregate `find` передаётся provider/revision boundary и проверяет cancellation; 7-секундный handoff callback не отменяет. Role объединяет canonical rights, а ambiguous short alias даёт `bad_value`. Injected service исполняет handlers на actor capability; production default dormant до Task 22.

Широкие `DEC.2026-08-23.V0-13-EXECUTION-SURFACE` и `DEC.2026-08-23.MODULE-CONTRACT` остаются `planned`; v0.12 и `CTR.WIRE.TOOL-SURFACE` до Task 22 не меняются.

**Почему.** Tasks 15–21 получают одну typed-reader, continuation и task policy до публичного cutover.

**Цена.** До Task 22 контракт crate-private; WebSocketClient source view — gap, find читает bounded BSL declaration shell, cursor не переживает restart.
