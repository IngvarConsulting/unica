---
id: DEC.2026-09-15.RUN-NAMES-READ-AS-LAYER-AND-DIRECTION
---

# Публичные операции и команды раннера имеют собственные имена

Имена команд раннера (`infobase.dump`, `infobase.restore`,
`infobase.configuration.export`) не обязаны совпадать с именами операций
`run`. Модуль выгрузок сверяет поле `command` конверта с именем команды
раннера.
