---
id: INV.APP.DIAGNOSTIC-FILTERED-RESULTS
check:
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_provider_selection_uses_registry_order_and_skips_inapplicable_providers
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_code_filters_do_not_select_execution_providers
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_result_assembly_filters_sorts_and_applies_one_global_limit
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_result_assembly_keeps_cross_provider_duplicates_and_metadata_focus_order
gap: https://github.com/IngvarConsulting/unica/issues/963
---

# Лимит диагностики применяется после отбора находок

Координатор запускает применимых поставщиков. Отбор по паре «поставщик + код»
фильтрует находки, но не выключает других поставщиков. Одинаковые коды
разных поставщиков остаются отдельными находками со своим происхождением.

Отбор по важности и диапазону выполняется до общего лимита. Порядок стабилен
для одинаковых находок: учитываются место, фокус, порядок поставщиков,
вид элемента, код и сообщение. Полное число подходящих элементов считается
до усечения; число возвращённых — после него.

Усечение ответа не означает неполноту анализа. Ошибки ресурсов считаются
отдельно, даже если лимит скрыл соответствующий элемент.
Проверки относятся к координатору и не вводят старые аргументы фильтрации
в публичную поверхность.
