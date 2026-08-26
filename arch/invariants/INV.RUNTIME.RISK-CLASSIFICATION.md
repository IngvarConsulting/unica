---
id: INV.RUNTIME.RISK-CLASSIFICATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/runtime_admission.rs::runtime_risk_classification_is_closed
scope: [app, product]
---

# Допуск applied runtime имеет закрытую классификацию риска

Каждая каноническая операция `unica.runtime.execute` получает именованную
категорию риска и предупреждение. Комбинация без проверенной классификации
отказывает кодом `runtime_operation_unbounded`, а не исполняется по умолчанию.
