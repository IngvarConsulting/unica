---
id: INV.ANALYSIS.CALL-GRAPH-ON-DEMAND
check: []
gap: https://github.com/IngvarConsulting/unica/issues/949
---

# Чтение метода не вычисляет граф без явного запроса

Обычный `unica.view` метода не запрашивает граф вызовов у анализатора.
Чтобы получить сводку графа вместе с методом, вызывающий явно включает
`callGraph` в `filter.sections`.

Чтение ветви `Caller` или `Callee` само является запросом графа.
