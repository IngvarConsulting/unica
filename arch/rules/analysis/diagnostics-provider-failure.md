---
id: INV.APP.DIAGNOSTIC-PROVIDERS
check:
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_resource_failure_keeps_incomplete_coverage_when_limit_hides_the_failure
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_concurrency_contains_provider_panic_and_keeps_sibling_items
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_concurrency_timeout_cancels_provider_without_waiting_for_it
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_outcome_matrix_distinguishes_complete_partial_and_failed
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_unmappable_observation_keeps_the_proven_findings_of_its_provider
  - crates/unica-coder/src/infrastructure/diagnostics.rs::resident_baseline_is_a_named_filter_and_a_broken_one_names_its_cause
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::bsl_check_passes_under_a_proven_baseline_and_names_a_broken_one
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_baseline_suppression_is_a_named_filter_not_incompleteness
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
и физические пути. Отказ `check` из-за неё передаёт эту причину, а не
сообщает только, что анализ не завершился. Подавление допустимо лишь
в секции с результатом: пустой ответ и отказ поставщика его не несут.

Проверки относятся к координатору, разбору ответа анализатора и вердикту
`check`. Канонический `unica.check` сейчас отказывает
при неполном анализе; правило не обещает выдачу частичных находок через этот вход.
