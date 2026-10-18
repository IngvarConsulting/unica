---
id: INV.RUNTIME.SERVICE-MUTATION-INVALIDATION
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::canonical_saved_apply_notifies_live_helpers_once_and_restarts_the_warmed_analyzer
  - crates/unica-coder/src/infrastructure/workspace_services.rs::failed_helper_invalidation_preserves_the_published_receipt_and_unknown_old_results
  - tests/ci/test_acceptance_scenarios.py::AcceptanceDeliveryCorpusRunTests.test_analyzer_delivery_wires_have_terminal_content
---

# Успешная правка уведомляет живые сервисы рабочего пространства

После успешного применения правки Unica передаёт события изменения живым
внутренним сервисам этого рабочего пространства. Они отмечают зависимое
состояние анализатора и индекса как изменившееся без участия ИИ.

Повторное получение результата уже применённой правки не отправляет события
заново. Сбой доставки уведомления не отменяет опубликованную правку.
Доступ к пригодному устаревшему индексу сохраняется по
[правилу RLM](../analysis/rlm-stale-results.md).
