---
id: INV.APP.DIAGNOSTIC-SUPPRESSION-EVIDENCE
check:
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
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_named_suppression_failure_cannot_claim_a_complete_section
  - crates/unica-coder/src/infrastructure/diagnostics_baseline.rs::cli_names_every_broken_baseline_it_reports_before_start
  - crates/unica-coder/src/infrastructure/diagnostics_baseline.rs::baseline_detail_hides_a_spaced_physical_path_whole
  - crates/unica-coder/src/infrastructure/internal_adapters.rs::analyze_names_a_broken_baseline_the_cli_refused_before_start
  - crates/unica-coder/src/infrastructure/diagnostics_jsonl.rs::done_baseline_is_a_named_filter_and_only_a_broken_one_is_incomplete
---

# Подавление диагностик сохраняет доказанные факты и границы покрытия

Базовая линия диагностик анализатора — фильтр по настройке пользователя,
а не неполнота. Секция называет источник `baseline`, число скрытых известных
находок (`known`) и оставленных новых (`new`). Число исправленных записей
(`resolved`) не публикуется: в анализе части проекта апстрим его не доказывает.
Пустой список под базовой линией остаётся результатом с фактом подавления,
а не «чисто» и не пустым ответом. `check` модуля под исправной базовой линией
выносит вердикт и называет подавление рядом с ним.

Состояние `partial` у апстрима означает лишь, что анализ покрыл не весь
проект; классификация найденного при этом точна. Базовая линия делает секцию
неполной только когда она сломана (`error`, непустой `errors`, `error_code`) или
классификация не состоялась (`partial` без `known`). Причина содержит код
и подробность анализатора со скрытыми секретами и физическими путями,
включая пути с пробелами. Резидентный ответ сообщает сломанную линию состоянием
`error`; CLI `analyze` завершается ошибкой до `start` и называет её в stderr.
Unica преобразует оба исхода в неполную секцию с причиной, а не
«поставщик недоступен».
Отказ `check` передаёт эту причину. Секция с названной причиной не может
объявить себя полной. Подавление допустимо лишь в секции с результатом:
секция `Empty` без результата и отказ поставщика его не несут.

Фильтр авторов также называется фактом: `by:authors` и положительный `count`,
подтверждённый поставщиком. Ноль или отсутствие счётчика не создают факт.
Идентичность авторов и сырые подсказки поставщика не публикуются. Несколько
действующих фильтров сохраняются одновременно; авторский счётчик не заменяет
baseline known/new. Публичный `check` сохраняет baseline item в
`data.suppressed` и добавляет рядом authors item.

Резидентный `out_of_scope=true` означает, что выбранный файл не покрыт
анализом: секция неполная, не Empty, и содержит безопасную именованную причину.
Доказанные находки и факты подавления в разрешённой области сохраняются.
Усечение самого анализа поставщиком и сломанная базовая линия остаются
неполнотой. Физический выход ресурса за разрешённую область — отдельное
нарушение адаптера: вся его секция теряет недостоверные находки и факты.

Закреплённый CLI JSONL bsl-analyzer 0.2.86 не сообщает счётчики авторов
и признак пропуска. При эффективном author filter `check` отказывает
с названной безопасной причиной и не выдумывает число. При анализе всего
source-root настроенный diff_base также делает покрытие недоказанным.
Модульный `--diff-filter` перекрывает diff_base: одна эта настройка не делает
проверку модуля неполной. Конфигурация берётся из source-root (или явного
config): TOML, затем .bsl-analyzer.json, затем .bsl-language-server.json.
Ошибка чтения старшего файла не разрешает перейти к младшему. Обычный полный
CLI анализ без этих фильтров сохраняет прежний вердикт.

Проверки относятся к координатору, разбору ответа анализатора и вердикту
`check`; они не вводят старые аргументы фильтрации в публичную поверхность.
[Отказ публичного check при неполном анализе](diagnostics-provider-failure.md)
не обещает выдачу частичных находок через этот вход.
