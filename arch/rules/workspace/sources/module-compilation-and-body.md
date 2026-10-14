---
id: INV.SOURCE.MODULE-COMPILATION-AND-BODY
check:
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_compilation_count_and_nested_guards_match_actual_ranges
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_text_spans_doc_comment_directive_declaration_and_end_without_preprocessor
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::crlf_method_text_keeps_the_source_line_endings
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_body_returns_the_source_text_byte_for_byte
  - crates/unica-coder/src/application/v13/view.rs::module_text_is_returned_whole_beside_the_projected_node_without_pages
  - crates/unica-coder/src/application/v13/view.rs::module_text_over_the_limit_is_refused_whole_with_structural_next
  - crates/unica-coder/src/application/v13/source_text.rs::source_text_limit_is_the_owner_decided_sixteen_kib
  - crates/unica-coder/src/application/v13/source_text.rs::oversized_method_refusal_routes_to_the_method_and_module_structure
  - crates/unica-coder/tests/v13_search_integration.rs::public_view_refuses_bsl_text_over_sixteen_kib_whole_with_structural_next
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::method_body_text_read_by_view_replaces_the_same_span_and_repeats_as_noop
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Тело BSL отдаётся исходным текстом

`Body` модуля возвращает текст модуля целиком, байт в байт как в файле.
Единственная нормализация: начальные U+FEFF в текст не входят. Разделители
строк, табуляция и отсутствие перевода строки в конце файла сохраняются.
Текст идёт отдельным текстовым блоком ответа, вне конверта JSON: номеров
строк, элементов коллекции и курсора у него нет.

`Method.<имя>.Body` возвращает текст метода. Он начинается с
документирующего комментария — подряд идущих строк `//` непосредственно
над директивой компиляции или объявлением; пустая строка комментарий
обрывает. Затем идут директива, объявление, тело и строка `КонецПроцедуры`
или `КонецФункции` включительно. Инструкции препроцессора вокруг метода
в текст не входят. Тот же текст с правкой, переданный `code.replace`
по тому же адресу, заменяет ровно этот участок.

Текст модуля или метода больше 16 КиБ UTF-8 не отдаётся. Ответ отказывает
`result_too_large` без частичного текста, а `next` ведёт к адресам `Region`
и `Method` этого модуля, у метода — сначала к самому методу.

Для `Method.<имя>.Body` на большом файле ещё нужен ограниченный по памяти
способ вычислить границы метода: сейчас чтение разбирает модуль целиком.
Этот разрыв учтён в [#1119](https://github.com/IngvarConsulting/unica/issues/1119).

`Compilation` описывает диапазоны и накопленные условия препроцессора,
включая вложенные условия и отрицание предыдущих ветвей для `Иначе`.
Директива метода, свойства модуля и условие препроцессора остаются разными
фактами при вычислении эффективных контекстов.

Проверки проходят построитель проекции, `view` над файлом и публичный MCP;
обратную запись текста метода проверяет план `code.replace`.
