---
id: INV.SOURCE.LOGICAL-BRANCH-COUNT
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/filter_criterion_content.rs::filter_criterion_content_distinguishes_empty_missing_and_partial_declarations
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

Ветви внешнего графа вызовов `Caller` и `Callee` узел метода называет
[без `count`](../../analysis/call-graph-on-demand.md): число соседей знает
только анализатор. Его сообщает `props.total` в [ответе самой ветви](../../analysis/call-graph-results.md),
и этот счёт может включать соседей, которым нельзя дать логический адрес.
