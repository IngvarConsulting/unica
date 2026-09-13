- Date: `2026-08-28`
- Status: `approved`
- Decision: `DEC.2026-08-28.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION`

# Request-level reconciliation effects для `unica.apply`

## Контекст

Текущий hidden router делит request на singleton family calls. Code и XDTO
локально восстанавливают `ops[i]` через `enumerate()` и сразу финализируют
effects. Такое поведение достаточно для одиночного planner test, но не задаёт
семантику одного cross-family request: промежуточный add, отменённый remove,
может оставить событие, а локальный индекс перестаёт указывать на исходную
операцию.

Active `DEC.2026-08-26.RETAINED-APPLY-EFFECT-PUBLICATION-SLICE` начинается уже
с готового `PlannedApplyEffects`: actor сохраняет его порядок и строит receipt.
Он не выбирает, какие промежуточные planner candidates соответствуют
финальному результату request, поэтому расширять его смысл задним числом
нельзя.

## Выбор

Request парсится один раз. Каждая операция получает неизменный глобальный
индекс, а family planners работают с ordered contiguous runs против одного
staged state. Planner возвращает provisional candidate вместе с точными
touched paths. Единый request-level finalizer сравнивает эти пути с admitted
preimage и финальным postimage всего request.

Finalizer применяет два шага в фиксированном порядке:

1. удаляет candidates без surviving final change;
2. дедуплицирует оставшиеся события в stable first-surviving-occurrence order.

Только после этого actor получает `PlannedApplyEffects`; действующий retained
publication contract продолжает отвечать за commit, receipt и cache report.

## Отвергнутые варианты

- Суммировать effects каждого singleton planner: публикует transient state и
  теряет атомарную семантику batch.
- Сначала дедуплицировать, затем фильтровать: отменённый первый duplicate может
  поглотить более поздний surviving candidate.
- Восстановить события только из byte diff после планирования: сохраняет
  postimage, но теряет семантический тип и порядок операции, которые знает
  planner.

## Реализация и доказательство

W2a до первого W1 merge вводит `IndexedPlanOperation<T>`, path-bound
`ProvisionalApplyEffect` и единственный request-level finalizer. RED matrix
покрывает inverse XDTO operations, interleaved families, исходный `ops[i]`,
transient duplicate, poison rollback и admission-sealed authorities.

Planned decision не устанавливает действующий инвариант без теста. При
реализации W2a создаётся active successor с именованным aggregate check и
производным `INV.APP.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION`. Body planned
записи сохраняется, а successor меняет только её lifecycle frontmatter на
`status: superseded` и заполняет `superseded-by`.
