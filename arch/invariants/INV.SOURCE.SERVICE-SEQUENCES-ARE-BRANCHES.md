---
id: INV.SOURCE.SERVICE-SEQUENCES-ARE-BRANCHES
status: active
governs: product
decision: DEC.2026-09-10.DECLARED-SERVICE-KINDS-GET-A-SUBJECT
check: crates/unica-coder/src/infrastructure/v13_read/tests.rs::declared_service_kinds_finally_get_their_subject
scope: [product, source]
---

# Последовательности служб адресуются, а не сворачиваются в строку

Шаблоны URL и операции служб приходят ветвями `URLTemplate` и `Operation`, их
методы и параметры — ветвями `Method` и `Parameter`. Каждый элемент имеет свой
адрес, и спросить один метод можно, не читая весь шаблон.
