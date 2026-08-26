---
id: INV.SOURCE.DEFAULT-SET-SELECTION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/source_roots.rs::main_source_set_wins_without_io
scope: [source]
---

# Набор `main` детерминированно выбирается по умолчанию

Чистая функция выбора набора исходников предпочитает явно названный `main`, не
обращаясь к файловой системе и не завися от порядка остальных наборов.
