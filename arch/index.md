<!-- ПОРОЖДАЕТСЯ scripts/arch/registry.py --write-index; руками не правится -->

# Индекс реестра

| Символ | Вид | Статус | Суть | Файл |
| --- | --- | --- | --- | --- |
| `CON.FORMAT.PLATFORM-XML-8-3-27` | контракт | active | Профиль Platform XML 8.3.27 и сохранение байтов | [contracts/CON.FORMAT.PLATFORM-XML-8-3-27.md](contracts/CON.FORMAT.PLATFORM-XML-8-3-27.md) |
| `CON.WIRE.PROTOCOL` | контракт | active | Три ревизии протокола и только реализованные возможности | [contracts/CON.WIRE.PROTOCOL.md](contracts/CON.WIRE.PROTOCOL.md) |
| `CON.WIRE.TOOL-SURFACE` | контракт | active | Ведомость поверхности порождается из бинаря | [contracts/CON.WIRE.TOOL-SURFACE.md](contracts/CON.WIRE.TOOL-SURFACE.md) |
| `DEC.2026-08-18.ADDRESS-GRAMMAR` | решение | active | Адрес — чередование `Вид.Имя` до любой глубины | [decisions/2026-08-18-address-grammar.md](decisions/2026-08-18-address-grammar.md) |
| `DEC.2026-08-18.ARCHITECTURE-RESET` | решение | active | Архитектура описывается заново | [decisions/2026-08-18-architecture-reset.md](decisions/2026-08-18-architecture-reset.md) |
| `DEC.2026-08-18.EIGHT-ENTRIES` | решение | active | Публичная поверхность — восемь входов | [decisions/2026-08-18-eight-entries.md](decisions/2026-08-18-eight-entries.md) |
| `DEC.2026-08-18.NO-FILE-DSL` | решение | active | Файловый DSL не входит в публичный контракт | [decisions/2026-08-18-no-file-dsl.md](decisions/2026-08-18-no-file-dsl.md) |
| `DEC.2026-08-18.NODE-OR-DATA` | решение | active | Адрес достаёт до именуемых узлов, множества передаются данными | [decisions/2026-08-18-node-or-data.md](decisions/2026-08-18-node-or-data.md) |
| `DEC.2026-08-18.REGISTRY-SHAPE` | решение | active | Три символических реестра, одна запись — один файл | [decisions/2026-08-18-registry-shape.md](decisions/2026-08-18-registry-shape.md) |
| `DEC.2026-08-18.RESULT-FORM` | решение | active | Результат — компактный JSON со слотовой дисциплиной | [decisions/2026-08-18-result-form.md](decisions/2026-08-18-result-form.md) |
| `INV.DOC.ARCHIVE-FROZEN` | инвариант | active | Замороженный слой не читается и не правится | [invariants/INV.DOC.ARCHIVE-FROZEN.md](invariants/INV.DOC.ARCHIVE-FROZEN.md) |
| `INV.DOC.SUPERPOWERS-BOUNDARY` | инвариант | active | Формы superpowers не входят в реестр | [invariants/INV.DOC.SUPERPOWERS-BOUNDARY.md](invariants/INV.DOC.SUPERPOWERS-BOUNDARY.md) |
| `INV.REGISTRY.SYMBOL-MATCHES-PATH` | инвариант | active | Символ записи выводится из её пути | [invariants/INV.REGISTRY.SYMBOL-MATCHES-PATH.md](invariants/INV.REGISTRY.SYMBOL-MATCHES-PATH.md) |
| `INV.SURFACE.ACCEPTANCE-UNCHANGED` | инвариант | active | Сужение публикации не сужает приём | [invariants/INV.SURFACE.ACCEPTANCE-UNCHANGED.md](invariants/INV.SURFACE.ACCEPTANCE-UNCHANGED.md) |
| `INV.SURFACE.NAMESPACE` | инвариант | active | Публичные инструменты живут в пространстве unica | [invariants/INV.SURFACE.NAMESPACE.md](invariants/INV.SURFACE.NAMESPACE.md) |
| `INV.SURFACE.PUBLISHED-ARGS-ARE-READ` | инвариант | active | Публикуется то, что обработчик читает | [invariants/INV.SURFACE.PUBLISHED-ARGS-ARE-READ.md](invariants/INV.SURFACE.PUBLISHED-ARGS-ARE-READ.md) |
