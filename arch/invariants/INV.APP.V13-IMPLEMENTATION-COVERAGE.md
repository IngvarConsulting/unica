---
id: INV.APP.V13-IMPLEMENTATION-COVERAGE
status: active
governs: product
decision: DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS
check: tests/ci/test_v013_implementation_coverage.py::test_record_covers_exactly_the_public_tools_and_run_dictionary
scope: [app, product, wire]
---

# Реализованность v0.13 имеет отдельное проверяемое покрытие

Каждый публичный инструмент и каждый закрытый режим `unica.run` имеет статус
реализованности, не выведенный из формы result envelope. Статус `supported`
допустим только вместе с исполняемым тестовым свидетельством. Режимы, которых
нет в реализации, остаются `unsupported`, даже если их имена присутствуют в
публичном словаре.
