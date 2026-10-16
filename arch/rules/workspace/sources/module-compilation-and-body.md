---
id: INV.SOURCE.MODULE-COMPILATION-AND-BODY
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::body_line_view_and_resolve_share_absolute_byte_exact_ranges
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::body_line_ranges_refuse_file_eof_and_method_boundary_crossings
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::body_line_apply_refuses_before_staging_and_can_has_no_writes
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::body_line_read_validates_the_tail_and_continuation_reopens_exact_bytes
  - crates/unica-coder/src/application/v13/source_text.rs::continuation_fits_the_real_ranged_envelope_at_each_host_budget
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::body_read_hint_survives_receipt_reopen_and_tampering_is_rejected
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::body_read_hint_survives_completed_task_reopen_without_rewriting
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_compilation_count_and_nested_guards_match_actual_ranges
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_text_spans_doc_comment_directive_declaration_and_end_without_preprocessor
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::crlf_method_text_keeps_the_source_line_endings
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_body_returns_the_source_text_byte_for_byte
  - crates/unica-coder/src/application/v13/view.rs::module_text_is_returned_whole_beside_the_projected_node_without_pages
  - crates/unica-coder/src/application/v13/view.rs::module_text_over_the_limit_is_refused_whole_with_structural_next
  - crates/unica-coder/src/application/v13/view.rs::oversized_module_text_does_not_refuse_a_projection_that_returns_no_text
  - crates/unica-coder/src/application/v13/source_text.rs::source_text_limit_is_the_largest_known_host_budget
  - crates/unica-coder/src/application/v13/source_text.rs::oversized_method_refusal_routes_to_the_method_and_module_structure
  - crates/unica-coder/src/application/v13/source_text.rs::response_budget_refuses_text_with_its_envelope_and_marks_oversized_branches
  - crates/unica-coder/src/application/v13/source_text.rs::streamed_module_text_keeps_memory_bounded_and_counts_the_whole_size
  - crates/unica-coder/src/application/v13/source_text.rs::streamed_module_text_strips_repeated_bom_across_chunks_and_keeps_a_partial_one
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_body_beyond_the_limit_is_still_checked_as_utf8_in_full
  - crates/unica-coder/src/application/v13/source_text.rs::overbudget_mark_and_refusal_share_one_boundary
  - crates/unica-coder/src/application/v13/source_text.rs::oversized_body_routes_by_address_segments_not_by_text
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::view_never_returns_module_text_above_the_largest_host_budget
  - crates/unica-coder/src/interfaces/task_projection.rs::task_result_with_source_text_follows_the_client_budget
  - crates/unica-coder/src/interfaces/mcp.rs::every_tool_that_can_return_source_text_declares_its_result_size
  - crates/unica-coder/src/interfaces/mcp.rs::client_name_comes_from_initialize_or_from_the_request_meta
  - crates/unica-bootstrap/src/host/response_budget.rs::known_hosts_get_their_measured_budgets_and_others_the_default
  - crates/unica-bootstrap/src/host/response_budget.rs::explicit_setting_then_client_name_then_launch_environment_then_default
  - crates/unica-coder/tests/v13_search_integration.rs::public_view_refuses_bsl_text_over_the_default_budget_whole_with_structural_next
  - crates/unica-coder/tests/v13_search_integration.rs::public_view_budget_belongs_to_the_client_and_the_summary_marks_the_body
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::method_body_text_read_by_view_replaces_the_same_span_and_repeats_as_noop
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::method_text_that_does_not_parse_is_refused_as_the_input_text
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::method_body_text_with_a_preprocessor_wrapper_is_refused_in_wrapped_and_plain_modules
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Тело BSL отдаётся исходным текстом

`Body` модуля возвращает текст модуля целиком, байт в байт как в файле.
Единственная нормализация: начальные U+FEFF в текст не входят. Разделители
строк, табуляция и отсутствие перевода строки в конце файла сохраняются.
Текст идёт отдельным текстовым блоком ответа, вне конверта JSON: номеров
строк внутри самого текста, элементов коллекции и курсора у него нет.

`Method.<имя>.Body` возвращает текст метода. Он начинается с
документирующего комментария — подряд идущих строк `//` непосредственно
над директивой компиляции или объявлением; пустая строка комментарий
обрывает. Затем идут директива, объявление, тело и строка `КонецПроцедуры`
или `КонецФункции` включительно. Инструкции препроцессора вокруг метода
в текст не входят.

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
метод, заменяет участок, который отдаёт `view`: от комментария до строки
окончания. Текст без объявления заменяет только строки между объявлением
и окончанием. Текст с объявлением, который не разбирается как BSL,
отклоняется `bad_value` до записи с указанием на сам текст; текст, где
объявлению предшествуют инструкции препроцессора, тоже: обёртка метода
в его текст не входит и правится по адресу модуля.

Ответ с текстом не превышает бюджета ответа клиента — по умолчанию 16 КиБ
UTF-8 вместе с конвертом. Известным хостам назначен наибольший бюджет,
который они надёжно передают модели целиком; больше наибольшего из них
текст не отдаётся никому. Бюджет задаёт явная настройка, затем имя клиента
из `initialize` или из `_meta` запроса без `initialize` (неизвестное имя —
умолчание), а без имени — окружение запуска плагина. Бюджет считает весь
ответ; конверт учитывается одним запасом и для отказа, и для пометки
в сводке модуля. Инструменты, чей ответ может нести текст (`unica.view`
и `unica.task.result`), объявляют хосту предел символов ответа; объявление
относится к любому их ответу, а не только к тексту `Body`. Ответ сверх бюджета
отказывает `result_too_large` без частичного текста. Если хотя бы одна
полная начальная строка помещается, `next` сначала предлагает диапазон
`Body#L<a>-L<b>` с наибольшим помещающимся начальным фрагментом; `a` —
абсолютное начало запрошенного Body или диапазона. Подсказка проверяется
по тому же конверту и бюджету, что реальный `view` этого адреса. Если первая
строка не помещается, ложной подсказки нет. Структурные `next` к `Region`
и `Method` сохраняются, у метода — и к самому методу. Предел касается только самого текста: контракт операции
`<узел>.Can.<op>` отдаётся и для большого модуля.

Для `Method.<имя>.Body` на большом файле ещё нужен ограниченный по памяти
способ вычислить границы метода: сейчас чтение разбирает модуль целиком.
Этот разрыв учтён в [#1119](https://github.com/IngvarConsulting/unica/issues/1119).

`Compilation` описывает диапазоны и накопленные условия препроцессора,
включая вложенные условия и отрицание предыдущих ветвей для `Иначе`.
Директива метода, свойства модуля и условие препроцессора остаются разными
фактами при вычислении эффективных контекстов.

Проверки проходят построитель проекции, `view` над файлом и публичный MCP;
обратную запись текста метода проверяет план `code.replace`.
