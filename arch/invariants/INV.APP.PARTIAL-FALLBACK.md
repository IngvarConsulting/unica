---
id: INV.APP.PARTIAL-FALLBACK
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/runtime_build_fallback.rs::completed_designer_partial_load_failure_is_retryable
scope: [app]
---

# Завершённый частичный отказ Designer классифицируется для повтора

Закрытая квитанция о завершившемся ошибкой частичном шаге Designer после
внешнего кода `4` возвращает доказательство повтора с ожидаемыми набором
исходников, числом файлов и внутренним кодом выхода.
