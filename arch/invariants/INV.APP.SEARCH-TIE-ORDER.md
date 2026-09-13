---
id: INV.APP.SEARCH-TIE-ORDER
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/documentation_retrieval.rs::ties_break_by_document_index_deterministically
scope: [app]
---

# Равные оценки документации упорядочены детерминированно

Попадания с одинаковой оценкой сохраняют устойчивый порядок индекса корпуса.
