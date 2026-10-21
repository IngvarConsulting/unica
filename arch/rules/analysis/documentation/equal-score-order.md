---
id: INV.APP.SEARCH-TIE-ORDER
check:
  - crates/unica-documentation-sources/src/documentation_retrieval.rs::ties_break_by_document_index_deterministically
---

# Равные оценки поиска сохраняют порядок документов в корпусе

Если документы получили одинаковую поисковую оценку, раньше выдаётся
документ с меньшим индексом в корпусе. Порядок не должен случайно меняться
между одинаковыми запросами.
