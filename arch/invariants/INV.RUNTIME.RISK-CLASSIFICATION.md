---
id: INV.RUNTIME.RISK-CLASSIFICATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/application/runtime_admission.rs::every_classified_applied_runtime_operation_is_warned_with_its_reason
  - crates/unica-coder/src/application/runtime_admission.rs::unclassified_applied_operation_still_fails_closed
  - crates/unica-coder/src/application/runtime_admission.rs::canonical_runtime_surface_has_an_explicit_risk_classification
scope: [app, product]
---

# Допуск applied runtime имеет закрытую классификацию риска

Каждая каноническая операция `unica.runtime.execute` получает именованную
категорию риска и предупреждение. Комбинация без проверенной классификации
отказывает кодом `runtime_operation_unbounded`, а не исполняется по умолчанию.
