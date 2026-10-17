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
  - crates/unica-coder/src/infrastructure/internal_adapters.rs::analyze_failure_names_where_the_run_stopped
  - crates/unica-coder/src/infrastructure/diagnostics.rs::backend_failure_keeps_where_the_run_stopped_and_drops_its_text
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::bsl_check_names_why_the_engine_gave_no_result_and_how_to_recover
  - crates/unica-coder/src/infrastructure/daemon/server.rs::module_check_names_why_the_analyzer_gave_no_result
  - crates/unica-coder/src/infrastructure/internal_adapters.rs::streaming_runner_reports_a_spawn_failure_as_such
  - crates/unica-coder/src/application/diagnostics.rs::absent_engine_reason_is_published_only_in_its_canonical_shape
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

Сломанная или не применённая базовая линия разбирается по
[правилу подавления и покрытия](diagnostics-suppression-evidence.md): её
безопасная причина сохраняется и в секции, и в отказе `check`.

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

Неполный анализ в отказе `check` всегда называет причину и действие
восстановления. Прогон анализатора без результата различает место остановки:
движка нет на машине, движок не прошёл проверку поставки, процесс
не запустился, превысил срок или завершился ошибкой. Вид определяется
местом остановки и состоянием диска, а не текстом ошибки. Каждый случай
получает свой код секции и фиксированную формулировку по коду; текст и stderr
движка наружу не выходят. Отсутствующий движок называет инструмент, цель
и следующий шаг по режиму установки, но не ожидаемый путь. Отсутствие,
непрошедшая проверка и незапуск движка уточняются `provider_absent`.
Если движка нет, `check` сначала запускает его доставку, как поиск с ролью,
и отвечает её состоянием.
