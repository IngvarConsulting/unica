---
id: INV.SOURCE.MODULE-COMPILATION
check:
  - crates/unica-address/src/address.rs::compilation_branches_are_retired_but_the_method_name_is_valid
  - crates/unica-bsl/src/projection.rs::serialized_module_projection_shape_is_stable
  - crates/unica-bindings-integration-tests/tests/projection.rs::method_compilation_count_and_nested_guards_match_actual_ranges
---

# Внутренние факты сохраняют условия препроцессора

Внутренние факты модуля сохраняют диапазоны и накопленные условия препроцессора,
включая вложенные условия и отрицание предыдущих ветвей для `Иначе`.
Директива метода, свойства модуля и условие препроцессора остаются разными
фактами при вычислении эффективных контекстов.

Вычисленные контексты и условия не публикуются в ответах модуля или метода.
Внешней ветви и адреса `Compilation` нет; исходные директивы и условия
остаются в тексте `Body`.
