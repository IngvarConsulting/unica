---
id: DEC.2026-08-31.V0-13-FIRST-IMPLEMENTATION-VERTICALS
status: superseded
governs: product
realized: tests/ci/test_v013_implementation_coverage.py::test_record_covers_exactly_the_public_tools_and_run_dictionary
supersedes: []
superseded-by: DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS
establishes: [INV.APP.V13-IMPLEMENTATION-COVERAGE]
design: docs/design/2026-08-31-v0-13-first-implementation-verticals-design.md
---

# v0.13 records implementation separately from result shape

**Решение.** Фактическая поддержка режимов восьми предметных инструментов и
трёх compatibility Task-инструментов фиксируется отдельным машиночитаемым
покрытием со статусами `supported`, `partial`, `unsupported`, `removed` и
исполняемым тестовым свидетельством. `contract: typed` в surface ledger не
используется как синоним реализованности.

Первая предметная вертикаль строит общий retained metadata apply planner и
логические read-проекции. В `run` доказан закрытый словарь операций;
`syntax.check` доказан как bounded durable Task: закрытые аргументы, timeout и
capture limits процесса, cancellation и безопасный terminal/provider result.
`query.execute` отсутствует в словаре v0.13 по отдельному решению
`DEC.2026-08-31.V0-13-NO-QUERY-EXECUTE`.

**Почему.** Стабильность формы результата и наличие предметного движка — разные
свойства. Их смешение завышает готовность миграции и скрывает остаток работ.

**Цена.** Любое расширение поддерживаемого режима обязано одновременно обновить
покрытие и предоставить исполняемое доказательство.
