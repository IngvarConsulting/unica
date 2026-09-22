---
id: INV.SOURCE.DEFAULT-SET-SELECTION
check:
  - crates/unica-coder/src/infrastructure/source_roots.rs::selects_the_sole_configuration
  - crates/unica-coder/src/infrastructure/source_roots.rs::rejects_ambiguous_configurations_without_main
  - crates/unica-coder/src/domain/source_roots.rs::main_source_set_wins_without_io
---

# Набор main имеет приоритет при выборе по умолчанию

Когда требуется выбрать набор исходников по умолчанию, явно названный `main`
имеет приоритет независимо от своего вида и порядка остальных наборов.
Для этого выбора не требуется обращаться к файловой системе.

Если `main` нет, выбирается единственный набор конфигурации. Несколько
конфигураций без `main` дают отказ вместо произвольного выбора.
Проверки проходят внутренний выбор корня и обнаружение наборов.
