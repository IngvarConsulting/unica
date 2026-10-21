---
id: INV.APP.PROVIDER-WORKER-CONCURRENCY
check:
  - crates/unica-code-intelligence/src/coordinator.rs::search_executes_all_workers_past_the_former_provider_quota
  - crates/unica-code-intelligence/src/coordinator.rs::read_executes_all_workers_past_the_former_provider_quota
  - crates/unica-code-intelligence/src/coordinator.rs::coordinator_enforces_budget_when_provider_ignores_deadline_and_cancellation
  - crates/unica-code-intelligence/src/coordinator.rs::read_coordinator_enforces_deadline_for_non_cooperative_provider
  - crates/unica-diagnostics/src/coordinator.rs::diagnostics_executes_all_workers_past_the_former_provider_quota
  - crates/unica-diagnostics/src/coordinator.rs::diagnostics_retains_each_noncooperative_worker_after_its_caller_returns
---

# Поставщики исполняют запросы без квоты конкуренции

Координаторы поиска, чтения кода и диагностики не отказывают в исполнении
из-за числа уже работающих исполнителей того же поставщика. Каждый запущенный
поток остаётся в общем для его координатора учёте до фактического завершения
и сбора потока. Завершение ожидания вызывающего не освобождает этого владельца.

Поиск и чтение кода используют общий учёт исполнителей; у диагностики свой
учёт. Снятие квоты не отменяет обработку ошибок запуска или паники поставщика.

Оставшиеся автоматические сроки координаторов снимаются в
[задаче #1119](https://github.com/IngvarConsulting/unica/issues/1119).
