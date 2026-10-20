---
id: INV.ANALYSIS.PARSER-ANALYZER-REVISION
check:
  - tests/ci/test_parser_revision.py::ParserRevisionTests.test_repository_parser_and_analyzer_sources_are_aligned
---

# Встроенный разбор BSL и поставляемый анализатор используют одну ревизию

Rust-зависимости `parser` и `syntax`, используемые Unica внутри процесса,
закреплены на том же исходном коммите `bsl-analyzer`, что и поставляемый
внешний анализатор. `Cargo.toml`, разрешённые зависимости `Cargo.lock`
и `sourceCommit` анализатора в `tools.lock.json` согласованы.

Это сохраняет общую версию разбора языка для чтения модулей и анализа.
Совпадение коммитов само по себе не доказывает равенство всех результатов.
