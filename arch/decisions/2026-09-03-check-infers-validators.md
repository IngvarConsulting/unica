---
id: DEC.2026-09-03.CHECK-INFERS-VALIDATORS
status: active
governs: product
realized: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
supersedes: []
superseded-by: null
establishes: [INV.SURFACE.CHECK-INFERS-VALIDATORS, INV.SOURCE.WRITABLE-PROFILE-GATE]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-09-03-check-infers-validators-design.md
---

# `unica.check` выбирает валидаторы сам

**Решение.** `unica.check` принимает только `at`. Валидаторы узла следуют из
его вида и фактов, которые уже знает порт чтения: корень набора-конфигурации
проверяет `cf`, корень набора-расширения — `cfe`, форма — `form`, макет —
`dcs` или `mxl` по `TemplateType` дескриптора, роль — `role`, подсистема —
`subsystem`, командный интерфейс подсистемы (узел `Interface`) — `interface`,
любой другой объект метаданных — типизированный валидатор `meta`. Узел без
валидаторов (модуль, метод, ветка) отвечает читаемостью. Ответ несёт
`status`, список `validators` и диагностики с полем `validator`.

**Что снято.** Аргумент `filter.validation.profile` и закрытый список
профилей на проводе. Профиль был следом от имени старого инструмента
`*.validate`: соответствие профиля виду узла было жёстким, и Unica уже
отказывала неверный выбор словами «pick the profile of `at`». Выбор,
который сервер вычисляет сам, не передаётся модели.

**Что это чинит.** Валидаторы расширения и командного интерфейса получают
свой файл (`ExtensionPath`, `CIPath`) из логического адреса внутри Unica;
до этого профили `cfe` и `interface` отвечали ошибкой аргумента и не были
исполнимы.

**Проверки.** Таблица вид → валидаторы закрыта юнит-тестом
`plan_for_node`; корпус приёмки замораживает `status` и `validators`
каждого шага `check` над узлом, а шаг с `filter` — типизированный отказ.
