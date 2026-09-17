---
id: DEC.2026-09-15.RUN-NAMES-READ-AS-LAYER-AND-DIRECTION
status: active
governs: product
realized: crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_names_read_as_layer_and_direction
supersedes: []
superseded-by: null
establishes: [INV.WIRE.RUN-NAMES-ARE-LAYER-AND-DIRECTION]
changes: [CTR.WIRE.TOOL-SURFACE, INV.SURFACE.RUN-INTENTS-DIRECTIONAL, INV.RUNTIME.V13-INFOBASE-EXPORTS]
design: docs/plans/2026-09-04-run-operations-requirements.md
---

# Имена словаря `run` читаются как слой и направление

**Решение.** Имя операции словаря `unica.run` — `<слой>.<глагол>`.
Существительное называет слой, с которым работает операция: `infobase` —
база целиком, `cf` — конфигурация или расширение в ней, `source` — исходники
рабочего пространства, `artifact` — собираемый файл, `client` — сеанс.
Глагол считается относительно базы: `export` — из базы наружу, `import` —
снаружи в базу; `create`, `build`, `run` называют действие без направления.
Одно направление не называется двумя словами: `dump`, `restore`, `load` из
словаря уходят. Состав после переименования: `infobase.create`,
`infobase.export`, `infobase.import`, `cf.export`, `cf.import`,
`source.export`, `source.import`, `artifact.build`, `client.run`.

**Что чинит.** До решения одно и то же направление было названо тремя парами:
`dump`/`restore` для DT, `export`/`load` для CF, `dump`/`build` для
исходников, а слой конфигурации прятался в `infobase.configuration.*`.
Владелец по имени не мог сказать, что произойдёт, — значит не сможет и
модель, для которой имя операции есть весь контекст выбора. Новое имя
читается без словаря: «положить правки исходников в базу» — слой `source`,
внутрь — `source.import`; «снять базу в DT» — слой `infobase`, наружу —
`infobase.export`.

**Чем держится.** Проверка каталога разбирает каждое имя на слой и глагол по
закрытым спискам, отвергает снятые глаголы и требует, чтобы у `export` был
парный `import` на том же слое. Имена команд раннера остаются его собственными
(`infobase.dump`, `infobase.restore`, `infobase.configuration.export`):
модуль выгрузок сверяет поле `command` конверта с именем команды раннера,
а не с именем операции словаря — совпадение имён было случайным.

**Цена.** Три реализованные операции меняют имя до первого пререлиза, когда
это ещё ничего не стоит снаружи: `infobase.configuration.export` → `cf.export`,
`infobase.dump` → `infobase.export`, `infobase.restore` → `infobase.import`.
Принятые записи о них (`INV.SURFACE.RUN-INTENTS-DIRECTIONAL`,
`INV.RUNTIME.V13-INFOBASE-EXPORTS`, `CTR.WIRE.TOOL-SURFACE`) переписаны
на новые имена этим же решением; решения-основания хранят прежние имена как
историю. Инвентарь паритета с v0.12 ведёт `operation=dump` в `source.export`,
`operation=build` в `source.import`, `operation=load` — без наследника, как и
раньше. Семейство расширений, когда войдёт в словарь, получит слой
`extension` и глаголы состояния (`list`, `info`, `create`, `delete`,
`activate`) — списки проверки расширяются тем решением.
