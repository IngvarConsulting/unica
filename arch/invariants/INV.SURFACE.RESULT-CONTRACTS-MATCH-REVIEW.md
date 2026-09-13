---
id: INV.SURFACE.RESULT-CONTRACTS-MATCH-REVIEW
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::tool_specs_match_reviewed_result_contracts
scope: [wire]
---

# Контракт результата совпадает с ревью поверхности

Каждый публичный инструмент, отмеченный в `arch/tool-surface-review.json` как
типизированный, объявляет типизированный контракт результата в живом реестре;
остальные явно остаются внешним потоком.
