---
id: INV.SOURCE.LOGICAL-BRANCH-COUNT
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::configuration_root_branch_counts_match_every_reachable_collection
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_capability_parents_expose_canonical_module_collections
---

# Число в ветви соответствует доступной через неё коллекции

Для ветвей типизированных исходников `count` обозначает полное количество достижимых элементов её
коллекции, а не число строк случайной страницы. Объявленная ветвь должна
читаться по своему адресу; один адрес ветви не объявляется дважды.

Проверки проходят все ветви корня конфигурации в фикстуре и коллекции модулей
конфигурации, документа и формы. Они не являются исчерпывающей проверкой
каждого предметного вида.

Ветви внешнего графа вызовов имеют [отдельный смысл счётчика](../../analysis/call-graph-results.md):
он может включать соседей, которым нельзя дать логический адрес.
