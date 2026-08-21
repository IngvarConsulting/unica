---
id: INV.APP.PARTIAL-FALLBACK
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/runtime_build_fallback.rs::completed_designer_partial_load_failure_is_retryable
scope: [app]
---

# Повтор разрешает только завершённый частичный отказ Designer

Классификатор допускает полный повтор после внешнего кода `4` только для
закрытой квитанции о завершившемся ошибкой частичном шаге Designer.
