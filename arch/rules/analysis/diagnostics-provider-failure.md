---
id: INV.APP.DIAGNOSTIC-PROVIDERS
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::bsl_check_preserves_only_canonical_jsonl_failure_recovery
  - tests/ci/test_acceptance_scenarios.py::AcceptanceFaultCorpusRunTests.test_controlled_jsonl_faults_preserve_public_reason_without_provider_input
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_resource_failure_keeps_incomplete_coverage_when_limit_hides_the_failure
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_concurrency_contains_provider_panic_and_keeps_sibling_items
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_concurrency_timeout_cancels_provider_without_waiting_for_it
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_outcome_matrix_distinguishes_complete_partial_and_failed
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_unmappable_observation_keeps_the_proven_findings_of_its_provider
  - crates/unica-coder/src/infrastructure/diagnostics.rs::resident_baseline_is_a_named_filter_and_a_broken_one_names_its_cause
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::bsl_check_passes_under_a_proven_baseline_and_names_a_broken_one
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_baseline_suppression_is_a_named_filter_not_incompleteness
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_named_suppression_failure_cannot_claim_a_complete_section
  - crates/unica-coder/src/infrastructure/diagnostics_baseline.rs::cli_names_every_broken_baseline_it_reports_before_start
  - crates/unica-coder/src/infrastructure/diagnostics_baseline.rs::baseline_detail_hides_a_spaced_physical_path_whole
  - crates/unica-coder/src/infrastructure/internal_adapters.rs::analyze_names_a_broken_baseline_the_cli_refused_before_start
---

# Сбой диагностического движка не стирает доказанные находки

Координатор сохраняет результаты исправных поставщиков при отказе соседа
и помечает общий результат как неполный. Паника даёт секцию `Failed`
с кодом `provider_panicked`. При исчерпании бюджета координатор отменяет
задержавшегося поставщика и сообщает `provider_timeout`, не ожидая его
самостоятельного завершения. Если полезного результата не дал никто,
итог — отказ; доказанная пустая выдача исправного поставщика считается результатом.

Если внутри разрешённой области нельзя доказать цель отдельной находки,
доказанные находки того же поставщика сохраняются, а секция становится неполной.
Ошибка обработки ресурса также не должна называться полным анализом.

Сломанная или не применённая базовая линия диагностик делает секцию неполной
и называет причину: код и подробность анализатора, в которых скрыты секреты
и физические пути, включая пути с пробелами. Резидентный ответ сообщает
такую линию состоянием `error`; CLI `analyze` завершается ошибкой до события
`start`, и причину называет его stderr. Обе формы дают одну и ту же
неполную секцию с причиной, а не «поставщик недоступен». Отказ `check`
из-за неё передаёт эту причину, а не сообщает только, что анализ
не завершился. Секция, назвавшая причину, не может объявить себя полной.
Подавление допустимо лишь в секции с результатом: пустой ответ и отказ
поставщика его не несут.

Проверки относятся к координатору, разбору ответа анализатора и вердикту
`check`. Канонический `unica.check` сейчас отказывает
при неполном анализе; правило не обещает выдачу частичных находок через этот вход.

Отказ `check` при `diagnostics_invalid` переносит каноническую безопасную
причину повреждения JSONL: строку, когда она установлена, и действие проверки
совместимости. Пустой поток называет отсутствие start без выдуманного номера.
Проекция принимает только целое сообщение существующего закрытого formatter;
похожий префикс, приписка, произвольная ошибка и исходный payload не публикуются.
Такой отказ остаётся `provider_unavailable`, а не бизнес-вердиктом `failed`
проверяемого BSL. Корпус с управляемым producer проверяет настоящий process
pipeline и весь публичный ответ; он не подменяет проверку опубликованного движка.
