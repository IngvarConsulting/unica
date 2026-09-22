---
id: INV.SOURCE.MODULE-COMPILATION-AND-BODY
check:
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_compilation_count_and_nested_guards_match_actual_ranges
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::explicit_body_preserves_lines_paginates_and_filters_without_method_duplication
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::crlf_projection_preserves_source_signature_but_body_lines_drop_record_terminators
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_body_context_filter_excludes_at_client_source_from_server_slice
---

# Срез BSL по контексту сохраняет исходные номера строк

Без фильтра `Body` возвращает исходные строки; явный `filter.context`
оставляет строки выбранного контекста исполнения. Номера строк не меняются,
страница не разрывает строку. Терминатор строки не входит в её поле `text`.

`Compilation` описывает диапазоны и накопленные условия препроцессора,
включая вложенные условия и отрицание предыдущих ветвей для `Иначе`.
Директива метода, свойства модуля и условие препроцессора остаются разными
фактами при вычислении эффективных контекстов.

Проверки проходят построитель проекции; отдельная проверка текущего `view`
исключает клиентский код из серверного среза.
