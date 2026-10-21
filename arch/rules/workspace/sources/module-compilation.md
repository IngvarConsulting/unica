---
id: INV.SOURCE.MODULE-COMPILATION
check:
  - crates/unica-bindings-integration-tests/tests/projection.rs::method_compilation_count_and_nested_guards_match_actual_ranges
---

# Compilation сохраняет диапазоны и накопленные условия препроцессора

`Compilation` описывает диапазоны и накопленные условия препроцессора,
включая вложенные условия и отрицание предыдущих ветвей для `Иначе`.
Директива метода, свойства модуля и условие препроцессора остаются разными
фактами при вычислении эффективных контекстов.
