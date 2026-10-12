---
id: INV.SOURCE.MODULE-COMPILATION-AND-BODY
check:
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_compilation_count_and_nested_guards_match_actual_ranges
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::explicit_body_preserves_lines_paginates_and_filters_without_method_duplication
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::crlf_projection_preserves_source_signature_but_body_lines_drop_record_terminators
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_body_returns_every_source_line_with_its_number
  - crates/unica-coder/src/application/v13/body_snapshot.rs::fragments_reassemble_utf8_line_and_preserve_crlf_boundaries
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Тело BSL сохраняет исходные строки и их номера

`Body` возвращает исходные строки модуля с их номерами; среза по контексту
исполнения у `view` нет. При чтении `Body` строка разбивается на фрагменты только
тогда, когда целиком не помещается в
ответ. Каждый фрагмент сохраняет исходный номер и байтовое смещение; клиент
восстанавливает строку в порядке смещений. Терминатор строки не входит в
`text`.

Для `Method.<имя>.Body` на большом файле ещё нужен ограниченный по памяти
способ вычислить диапазоны метода.
Этот разрыв учтён в [#1119](https://github.com/IngvarConsulting/unica/issues/1119).

`Compilation` описывает диапазоны и накопленные условия препроцессора,
включая вложенные условия и отрицание предыдущих ветвей для `Иначе`.
Директива метода, свойства модуля и условие препроцессора остаются разными
фактами при вычислении эффективных контекстов.

Проверки проходят построитель проекции; отдельная проверка текущего `view`
сверяет строки `Body` формы с исходным файлом.
