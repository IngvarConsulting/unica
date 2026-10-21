---
id: INV.WIRE.SOURCE-TEXT-RESPONSE-BUDGET
check:
  - crates/unica-execution-contract/src/source_text_budget.rs::continuation_fits_the_real_ranged_envelope_at_each_host_budget
  - crates/unica-execution/src/receipt_store/tests.rs::body_read_hint_survives_receipt_reopen_and_tampering_is_rejected
  - crates/unica-execution/src/task_store.rs::body_read_hint_survives_completed_task_reopen_without_rewriting
  - crates/unica-coder/src/application/v13/view.rs::module_text_over_the_limit_is_refused_whole_with_structural_next
  - crates/unica-coder/src/application/v13/view.rs::oversized_module_text_does_not_refuse_a_projection_that_returns_no_text
  - crates/unica-coder/src/application/v13/source_text.rs::source_text_limit_is_the_largest_known_host_budget
  - crates/unica-execution-contract/src/source_text_budget.rs::oversized_method_refusal_routes_to_the_method_and_module_structure
  - crates/unica-execution-contract/src/source_text_budget.rs::response_budget_refuses_text_with_its_envelope_and_marks_oversized_branches
  - crates/unica-execution-contract/src/source_text_budget.rs::overbudget_mark_and_refusal_share_one_boundary
  - crates/unica-execution-contract/src/source_text_budget.rs::oversized_body_routes_by_address_segments_not_by_text
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::view_never_returns_module_text_above_the_largest_host_budget
  - crates/unica-coder/src/interfaces/task_projection.rs::task_result_with_source_text_follows_the_client_budget
  - crates/unica-coder/src/interfaces/mcp.rs::every_tool_that_can_return_source_text_declares_its_result_size
  - crates/unica-coder/src/interfaces/mcp.rs::client_name_comes_from_initialize_or_from_the_request_meta
  - crates/unica-bootstrap/src/host/response_budget.rs::known_hosts_get_their_measured_budgets_and_others_the_default
  - crates/unica-bootstrap/src/host/response_budget.rs::zcode_measured_budget_preserves_client_and_environment_precedence
  - crates/unica-bootstrap/src/host/response_budget.rs::explicit_setting_then_client_name_then_launch_environment_then_default
  - crates/unica-coder/tests/v13_search_integration.rs::public_view_refuses_bsl_text_over_the_default_budget_whole_with_structural_next
  - crates/unica-coder/tests/v13_search_integration.rs::public_view_budget_belongs_to_the_client_and_the_summary_marks_the_body
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::body_line_read_validates_the_tail_and_continuation_reopens_exact_bytes
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_body_beyond_the_limit_is_still_checked_as_utf8_in_full
---

# Текст исходника помещается в бюджет ответа клиента

Ответ с текстом не превышает бюджета ответа клиента — по умолчанию 16 КиБ
UTF-8 вместе с конвертом. Известным хостам назначен наибольший бюджет,
который они надёжно передают модели целиком; больше наибольшего из них
текст не отдаётся никому. Бюджет задаёт явная настройка, затем имя клиента
из `initialize` или из `_meta` запроса без `initialize` (неизвестное имя —
умолчание), а без имени — окружение запуска плагина. Бюджет считает весь
ответ; конверт учитывается одним запасом и для отказа, и для пометки
в сводке модуля. Инструменты, чей ответ может нести текст (`unica.view`
и `unica.task.result`), объявляют хосту предел символов ответа; объявление
относится к любому их ответу, а не только к тексту `Body`.

Ответ сверх бюджета отказывает `result_too_large` без частичного текста.
Превышение бюджета не отменяет
[проверку UTF-8 всего исходного файла](../workspace/sources/module-compilation-and-body.md).
Если хотя бы одна полная начальная строка помещается, `next` сначала
предлагает диапазон `Body#L<a>-L<b>` с наибольшим помещающимся начальным
фрагментом; `a` — абсолютное начало запрошенного Body или диапазона.
Подсказка проверяется по тому же конверту и бюджету, что реальный `view`
этого адреса. Если первая строка не помещается, ложной подсказки нет.
Структурные `next` к `Region` и `Method` сохраняются, у метода — и к самому
методу. Предел касается самого текста: контракт операции `<узел>.Can.<op>`
отдаётся и для большого модуля.
