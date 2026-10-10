---
id: INV.APP.DIAGNOSTIC-FILTERED-RESULTS
check:
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_resource_failure_keeps_incomplete_coverage_when_limit_hides_the_failure
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_provider_selection_uses_registry_order_and_skips_inapplicable_providers
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_code_filters_do_not_select_execution_providers
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_result_assembly_filters_sorts_and_applies_one_global_limit
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_result_assembly_keeps_cross_provider_duplicates_and_metadata_focus_order
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_baseline_suppression_is_a_named_filter_not_incompleteness
  - crates/unica-coder/src/infrastructure/diagnostics_baseline.rs::baseline_verdict_is_a_filter_unless_broken_or_unclassified
  - crates/unica-coder/src/infrastructure/diagnostics.rs::resident_baseline_is_a_named_filter_and_a_broken_one_names_its_cause
  - crates/unica-coder/src/infrastructure/diagnostics.rs::resident_filters_preserve_captured_baseline_and_authors_together
  - crates/unica-coder/src/infrastructure/diagnostics.rs::resident_analysis_scope_is_incomplete_without_erasing_proven_findings_or_filters
  - crates/unica-coder/src/infrastructure/diagnostics.rs::resident_filter_absence_and_zero_are_clean_but_malformed_flags_are_rejected
  - crates/unica-coder/src/infrastructure/diagnostics_filters.rs::effective_cli_config_matches_pinned_precedence_and_module_scope
  - crates/unica-coder/src/infrastructure/diagnostics_filters.rs::cli_missing_filter_counts_preserves_findings_baseline_and_failure_states
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_authors_and_baseline_survive_named_analysis_incompleteness
  - tests/ci/test_acceptance_scenarios.py::AcceptanceDeliveryCorpusRunTests.test_analyzer_delivery_wires_have_terminal_content
  - tests/ci/test_acceptance_scenarios.py::AcceptanceDeliveryCorpusRunTests.test_pinned_resident_reports_real_baseline_author_and_scope_facts
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::bsl_check_passes_under_a_proven_baseline_and_names_a_broken_one
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

Базовая линия диагностик анализатора — фильтр по настройке пользователя,
а не неполнота. Известные находки, которые она скрыла, секция поставщика
называет фактом подавления: источник `baseline`, число скрытых известных
(`known`) и оставленных новых (`new`). Число исправленных записей (`resolved`)
не публикуется: в анализе части проекта апстрим его не доказывает. Пустой
список под базовой линией — результат с фактом подавления, а не «чисто»
и не пустой ответ. `check` модуля под исправной базовой линией выносит
вердикт и называет подавление рядом с ним.

Состояние `partial` у апстрима означает лишь, что анализ покрыл не весь
проект; классификация найденного при этом точна. Неполной базовая линия
делает секцию, только когда она сломана (`error`, непустой `errors`,
`error_code`) или классификация не состоялась (`partial` без `known`).

Проверки относятся к координатору, разбору ответа анализатора и вердикту
`check`; они не вводят старые аргументы фильтрации в публичную поверхность.

Пользовательский фильтр авторов также называется фактом: `by:authors` и
положительный `count`, который поставщик подтвердил. Ноль или отсутствие
счётчика не создают факт. Идентичность авторов и сырые подсказки поставщика
не публикуются. Несколько действующих фильтров сохраняются одновременно;
авторский счётчик не заменяет baseline known/new. Публичный `check` сохраняет
прежний baseline item в `data.suppressed` и добавляет рядом authors item.

Резидентный `out_of_scope=true` означает, что выбранный файл не был покрыт
анализом: секция неполная, не Empty, и содержит безопасную именованную причину.
Доказанные находки и факты подавления в разрешённой области сохраняются.
Усечение и сломанная базовая линия остаются неполнотой. Физический выход
ресурса за разрешённую область — отдельное нарушение адаптера: вся его
секция по-прежнему теряет недостоверные находки и факты.

Закреплённый CLI JSONL bsl-analyzer 0.2.86 не сообщает счётчики авторов и
признак пропуска. При эффективном author filter `check` отказывает с названной
безопасной причиной и не выдумывает число. При анализе всего source-root
настроенный diff_base также делает покрытие недоказанным. Модульный
`--diff-filter` перекрывает diff_base: одна эта настройка не делает проверку
модуля неполной. Конфигурация берётся из source-root (или явного config):
TOML, затем .bsl-analyzer.json, затем .bsl-language-server.json. Ошибка чтения
старшего файла не разрешает перейти к младшему. Обычный полный CLI анализ
без этих фильтров сохраняет прежний вердикт.
