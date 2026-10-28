---
id: INV.SOURCE.MODULE-COMPILATION-AND-BODY
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::body_line_view_and_resolve_share_absolute_byte_exact_ranges
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::body_line_ranges_refuse_file_eof_and_method_boundary_crossings
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::body_line_apply_refuses_before_staging_and_can_has_no_writes
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::body_line_read_validates_the_tail_and_continuation_reopens_exact_bytes
  - crates/unica-bindings-integration-tests/tests/projection.rs::method_text_spans_doc_comment_directive_declaration_and_end_without_preprocessor
  - crates/unica-bindings-integration-tests/tests/projection.rs::crlf_method_text_keeps_the_source_line_endings
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_body_returns_the_source_text_byte_for_byte
  - crates/unica-coder/src/application/v13/view.rs::module_text_is_returned_whole_beside_the_projected_node_without_pages
  - crates/unica-coder/src/application/v13/source_text.rs::streamed_module_text_keeps_memory_bounded_and_counts_the_whole_size
  - crates/unica-coder/src/application/v13/source_text.rs::streamed_module_text_strips_repeated_bom_across_chunks_and_keeps_a_partial_one
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_body_beyond_the_limit_is_still_checked_as_utf8_in_full
  - crates/unica-coder/tests/platform/code_patch_composition.rs::method_body_text_read_by_view_replaces_the_same_span_and_repeats_as_noop
  - crates/unica-coder/tests/platform/code_patch_composition.rs::method_text_that_does_not_parse_is_refused_as_the_input_text
  - crates/unica-coder/tests/platform/code_patch_composition.rs::method_body_text_with_a_preprocessor_wrapper_is_refused_in_wrapped_and_plain_modules
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Тело BSL отдаётся исходным текстом

`Body` модуля возвращает текст модуля целиком, байт в байт как в файле.
Единственная нормализация: начальные U+FEFF в текст не входят. Разделители
строк, табуляция и отсутствие перевода строки в конце файла сохраняются.
Текст идёт отдельным текстовым блоком ответа, вне конверта JSON: номеров
строк внутри самого текста, элементов коллекции и курсора у него нет.

`Method.<имя>.Body` возвращает текст метода, включая прикреплённый
документирующий комментарий при наличии: подряд идущие строки `//` непосредственно
над аннотацией, директивой компиляции или объявлением; пустая строка комментарий
обрывает. Затем идут аннотации, директива, объявление, тело и строка `КонецПроцедуры`
или `КонецФункции` включительно. Инструкции препроцессора вокруг метода
в текст не входят. Диапазон `Method.<имя>` в `resolve` начинается с аннотации,
директивы или объявления без документирующего комментария; различие задаёт
[контракт расположения исходника](../../mcp/resolved-source-location.md).

Явный селектор `Body#L<n>` или `Body#L<a>-L<b>` читает только указанные
полные строки с теми же разделителями. Нумерация абсолютная, как в поиске
и `resolve`, включая `Method.<имя>.Body`. Узел возвращает `props.lines`
в формате `{state: range, from, to}`; `resolve` называет тот же диапазон
и файл. Выход за EOF или за границы выбранного Body отказывает `bad_value`.
Чтение фрагмента проверяет UTF-8 всего файла и сохраняет отмену и проверку
идентичности исходника. Адрес с диапазоном предназначен для чтения:
`code.replace` и `code.insert` отказывают до staging; `next` ведёт к целому
содержащему методу, если весь диапазон внутри него, иначе к Body модуля.

`code.replace` по адресу `Method.<имя>.Body` с текстом, который объявляет
метод, заменяет участок, который отдаёт `view`. Текст без объявления заменяет
только строки между объявлением и окончанием. Текст с объявлением, который не разбирается как BSL,
отклоняется `bad_value` до записи с указанием на сам текст; текст, где
объявлению предшествуют инструкции препроцессора, тоже: обёртка метода
в его текст не входит и правится по адресу модуля.

Размер и отказ без частичного текста задаёт
[бюджет текстового ответа](../../mcp/source-text-response-budget.md),
условия препроцессора — [внутренние факты компиляции](module-compilation.md).

Для `Method.<имя>.Body` на большом файле ещё нужен ограниченный по памяти
способ вычислить границы метода: сейчас чтение разбирает модуль целиком.
Этот разрыв учтён в [#1119](https://github.com/IngvarConsulting/unica/issues/1119).
