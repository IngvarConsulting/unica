---
id: DEC.2026-08-28.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION
status: superseded
governs: product
realized: null
supersedes: []
superseded-by: DEC.2026-09-01.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION
establishes: []
design: docs/design/2026-08-28-request-level-apply-effect-reconciliation-design.md
---

# Request-level apply выводит effects из финального staged postimage

**Решение.** Один `unica.apply` один раз индексирует исходный поток операций и
передаёт family planners только упорядоченные операции с сохранённым глобальным
`ops[i]`. Planners изменяют единый staged state и возвращают path-bound
provisional effect candidates; превращать их в `PlannedApplyEffects` может
только request-level finalizer.

Finalizer сначала исключает candidate, если связанные с ним пути не изменены в
финальном postimage относительно admitted preimage, и только затем выполняет
stable first-surviving-occurrence dedup. Промежуточный effect от операции,
полностью отменённой последующей операцией того же request, не публикуется.

**Почему.** Суммирование singleton results выдаёт transient state за результат
транзакции, а dedup до postimage-фильтрации позволяет исчезнувшему первому
duplicate подавить более поздний surviving effect. Глобальный индекс нужен,
чтобы cross-family ошибка сохраняла исходный адрес запроса.

**Цена.** W2a вводит indexed wrapper, provisional effect evidence и единый
finalizer до parallel W1 fan-out. Пока направление не реализовано и не получило
active successor с именованной проверкой, существующий request router остаётся
provisional, а production V12 не меняется.
