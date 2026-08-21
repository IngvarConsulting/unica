---
id: INV.SOURCE.EXACT-VERSION-LITERAL
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/format_profile.rs::rejects_numeric_equivalents_of_the_exact_supported_literal
scope: [source]
---

# Поддерживаемая версия — точный литерал

Поддерживается только точный литерал версии `2.20`. Численно равные написания
`2.20.0`, `02.20` и `2.020` отклоняются как недопустимое свидетельство, а
числовое сравнение не канонизирует их в поддерживаемую версию.
