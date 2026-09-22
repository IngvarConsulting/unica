---
id: INV.SOURCE.CALL-GRAPH-STATE-IS-NAMED
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::the_branch_points_outward_and_names_what_it_could_not_address
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::the_method_node_carries_the_count_and_names_the_state
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::an_indexing_graph_names_itself_instead_of_printing_a_zero
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::one_unready_direction_decides_the_whole_state
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
