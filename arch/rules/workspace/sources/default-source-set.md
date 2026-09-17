---
id: INV.SOURCE.DEFAULT-SET-SELECTION
check:
  - crates/unica-coder/src/domain/source_roots.rs::main_source_set_wins_without_io
---

# Набор main имеет приоритет при выборе по умолчанию

Когда требуется выбрать набор исходников по умолчанию, явно названный `main`
имеет приоритет независимо от своего вида и порядка остальных наборов.
Для этого выбора не требуется обращаться к файловой системе.
