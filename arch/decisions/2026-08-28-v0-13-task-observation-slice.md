---
id: DEC.2026-08-28.V0-13-TASK-OBSERVATION-SLICE
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: []
changes: [CTR.APP.DAEMON-LONG-WORK-CAPABILITIES]
design: docs/design/2026-08-28-v0-13-completion-wavefront-design.md
---

# v0.13 ограничивает durable observation состоянием Task

**Решение.** В v0.13 progress observation уже materialized Task выражается
изменением status и `updatedAt`, наблюдаемым polling через native либо
compatibility projection одной durable Invocation. Действующие envelope, TTL,
poll interval и terminal result/failure сохраняются. Отдельный публичный
progress/log API не создаётся. Resume допустим только для класса с
типизированным зарегистрированным owner; в остальных случаях остаточный
`Working` после restart получает доказанный terminal outcome без повторного
предметного исполнения.

**Почему.** Этого достаточно для завершения выбранной поверхности 8/11 и
exactly-once handoff. Универсальный progress stream, хранение непрозрачных
аргументов и blind replay расширили бы wire, persistence и security contracts
без обязательного сценария v0.13.

**Цена.** После receipt клиент наблюдает состояние polling. Потерянный receipt
после финального submit deadline остаётся известным residual до отдельного
решения об idempotency token. Решение не добавляет resume owner ни одному
классу само по себе.
