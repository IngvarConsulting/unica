---
id: INV.APP.DIAGNOSTIC-FILTERED-RESULTS
check:
  - crates/unica-diagnostics/src/coordinator.rs::diagnostics_resource_failure_keeps_incomplete_coverage_when_limit_hides_the_failure
  - crates/unica-diagnostics/src/coordinator.rs::diagnostics_provider_selection_uses_registry_order_and_skips_inapplicable_providers
  - crates/unica-diagnostics/src/coordinator.rs::diagnostics_code_filters_do_not_select_execution_providers
  - crates/unica-diagnostics/src/coordinator.rs::diagnostics_result_assembly_filters_sorts_and_applies_one_global_limit
  - crates/unica-diagnostics/src/coordinator.rs::diagnostics_result_assembly_keeps_cross_provider_duplicates_and_metadata_focus_order
---

# Лимит диагностики применяется после отбора находок

Координатор запускает применимых поставщиков. Отбор по паре «поставщик + код»
фильтрует находки, но не выключает других поставщиков. Одинаковые коды
разных поставщиков остаются отдельными находками со своим происхождением.

Отбор по важности и диапазону выполняется до общего лимита. Порядок стабилен
для одинаковых находок: учитываются место, фокус, порядок поставщиков,
вид элемента, код и сообщение. Полное число подходящих элементов считается
до применения лимита; число возвращённых — после него.

Ограничение готовой выдачи координатором не означает неполноту анализа
поставщика. Ошибки ресурсов считаются отдельно, даже если лимит скрыл
соответствующий элемент.

Факты пользовательского подавления и границы доказанного покрытия описаны
[отдельно](diagnostics-suppression-evidence.md). Эти проверки координатора
не вводят старые аргументы фильтрации в публичную поверхность.
