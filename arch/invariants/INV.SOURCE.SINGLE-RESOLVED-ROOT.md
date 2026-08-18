---
id: INV.SOURCE.SINGLE-RESOLVED-ROOT
status: active
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/source_roots.rs
scope: [source]
---

# Корень исходников выбирается детерминированно и один раз

Все потребители корня исходников получают один и тот же ответ на одном пространстве, и корень
отделён от рабочего пространства как отдельное понятие.
