---
id: INV.SOURCE.SINGLE-RESOLVED-ROOT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/source_roots.rs::main_source_set_wins_without_io
scope: [source]
---

# Корень исходников выбирается детерминированно и один раз

Все потребители корня исходников получают один и тот же ответ на одном пространстве, и корень
отделён от рабочего пространства как отдельное понятие.
