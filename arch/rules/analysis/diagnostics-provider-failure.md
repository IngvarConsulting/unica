---
id: INV.APP.DIAGNOSTIC-PROVIDERS
check:
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_resource_failure_keeps_incomplete_coverage_when_limit_hides_the_failure
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_concurrency_contains_provider_panic_and_keeps_sibling_items
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_concurrency_timeout_cancels_provider_without_waiting_for_it
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_outcome_matrix_distinguishes_complete_partial_and_failed
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_unmappable_observation_keeps_the_proven_findings_of_its_provider
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

Проверки относятся к координатору. Канонический `unica.check` сейчас отказывает
при неполном анализе; правило не обещает выдачу частичных находок через этот вход.
