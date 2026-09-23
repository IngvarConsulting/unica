---
id: INV.SOURCE.CALL-GRAPH-STATE-IS-NAMED
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::the_branch_points_outward_and_names_what_it_could_not_address
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::the_method_node_carries_the_count_and_names_the_state
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::an_indexing_graph_names_itself_instead_of_printing_a_zero
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::one_unready_direction_decides_the_whole_state
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::call_graph_distinguishes_a_recursive_call_from_a_capped_answer
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::a_capped_graph_requires_a_complete_same_revision_snapshot_before_paging
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::capped_graph_refetches_exact_total_before_publishing_a_branch
  - crates/unica-coder/src/application/v13/view.rs::supplied_graph_collection_uses_stable_pages_bound_to_its_owner
---

# Граф вызовов показывает готовность и неполноту результата

Узел метода со сводкой графа содержит `props.callGraph`: `ready`, `indexing`
или `unavailable`. Состояние узла учитывает оба направления; готовность
одного не скрывает недоступность другого. Счётчик выводится только когда
известен, и ветвь `Caller` или `Callee` объявляется только с этим счётчиком.
Ноль означает известное отсутствие вызовов, а не ожидание индекса.

Элемент ветви содержит логический адрес соседнего метода и происхождение
связи: `resolved` или `inferred`. Если адрес соседа определить нельзя,
он учитывается в `limits` с количеством и непустой причиной. Внутреннее
имя анализатора не подставляется вместо логического адреса.

Ветви `Caller` и `Callee` содержат только связи вызова. Если анализатор
ограничил число соседей, Unica запрашивает полный снимок и проверяет его
полноту и ревизию до выдачи первой страницы. Недоступный полный снимок не
выдаётся за завершённую ветвь: ответ называет `indexing` или `unavailable`
без страницы и курсора. `limit` и курсор используют обычные страницы
`view`; продолжение читает сохранённый снимок графа, пока ревизия исходников
не изменилась. `props.graphRevision` называет ревизию графа, с которой он снят.
Рекурсивный вызов включает сам метод в элементы и счёт ветви.
