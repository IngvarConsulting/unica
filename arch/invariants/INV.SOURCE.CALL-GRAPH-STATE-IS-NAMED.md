---
id: INV.SOURCE.CALL-GRAPH-STATE-IS-NAMED
status: active
governs: product
decision: DEC.2026-09-11.CALL-GRAPH-BRANCHES-POINT-OUTWARD
check: crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::the_branch_points_outward_and_names_what_it_could_not_address
scope: [product, source]
---

# Состояние графа вызовов называется, а не угадывается по пустоте

Узел метода со сводкой графа несёт закрытое `props.callGraph`: `ready`,
`indexing` или `unavailable`. Счёт печатается только когда известен, а ветвь
объявляется только со счётом. Элемент ветви несёт адрес чужого метода и
происхождение ребра; сосед, которого нечем адресовать, посчитан в `limits` с
причиной, а не выброшен.
