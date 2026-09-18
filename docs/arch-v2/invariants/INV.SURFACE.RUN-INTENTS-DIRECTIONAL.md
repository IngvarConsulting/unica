---
id: INV.SURFACE.RUN-INTENTS-DIRECTIONAL
---

# Аргументы runtime-выгрузок публикуются закрытыми схемами

Реализованная операция публикует закрытую `argsSchema`; нереализованная
не выдаёт предположение о ещё не принятом контракте аргументов.

`cf.export` принимает `state`, workspace-relative `output` и необязательное
имя `extension`; `infobase.export` принимает только workspace-relative
`output`. Обе схемы закрыты и не передают модели выбор provider.
