---
id: DEC.2026-09-01.V0-13-REFUSAL-DISCIPLINE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_refusals_answer_one_diagnostics_channel_from_the_closed_code_set
supersedes: []
superseded-by: null
establishes: [INV.WIRE.V13-REFUSAL-CHANNEL, CTR.WIRE.COMPATIBILITY-TASK-TOOLS]
changes: [CTR.WIRE.COMPATIBILITY-TASK-TOOLS]
design: docs/design/2026-09-01-v0-13-refusal-discipline-design.md
---

# Отказ канонической поверхности отвечает одним каналом и ведёт к восстановлению

**Решение.** Каждый отказ инструмента v0.13 — предметного и compatibility Task —
отвечает конвертом с `diagnostics[]`, где первый элемент несёт закрытый `code`
и непустой `message`; отдельного канала `data.code` нет. Конфликт `ifRev`
получает собственный код `stale_revision` и называет обе ревизии — ожидаемую и
допущенную. Отказ по отсутствию обязательного аргумента с закрытым доменом
значений перечисляет этот домен. Сырая ошибка ОС не пересекает границу провайдера: живой
сбой ввода-вывода оборачивается фазой поиска, а отсутствие или исчезновение
файла на обходе допущенного логического scope — это пустой результат, не отказ.

**Почему.** Стратегию восстановления агент выбирает по коду, не по прозе:
«перечитай `rev` и повтори» и «провайдер недоступен» требовали одного взгляда в
исходники, потому что оба шли как `provider_unavailable`, а task-инструменты
прятали код в `data.code`. Голый `No such file or directory (os error 2)` на
валидном scope не оставлял ни причины, ни следующего шага.

**Цена.** Форма task-отказов меняется без совместимости с v0.13-препубликацией
(`CTR.WIRE.COMPATIBILITY-TASK-TOOLS` v2); строковый канал допуска apply сужен
типизированной ошибкой, и новые случаи конфликтов обязаны расширять её, а не
возвращаться к строке.
