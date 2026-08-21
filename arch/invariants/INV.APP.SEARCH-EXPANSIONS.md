---
id: INV.APP.SEARCH-EXPANSIONS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/documentation_retrieval.rs::expansions_add_terms_without_penalizing_originals
scope: [app]
---

# Расширения запроса не ослабляют исходные термы

Лексическое ядро добавляет термы расширения, сохраняя исходное совпадение и
не заменяя его менее точным вариантом.
