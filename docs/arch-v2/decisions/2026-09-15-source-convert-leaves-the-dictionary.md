---
id: DEC.2026-09-15.SOURCE-CONVERT-LEAVES-THE-DICTIONARY
status: active
governs: product
realized: crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_names_no_operation_that_needs_edt
supersedes: []
superseded-by: null
establishes: [INV.WIRE.RUN-STAYS-IN-THE-FORMAT-UNICA-READS]
changes: [CTR.WIRE.TOOL-SURFACE, INV.APP.V13-RUN-DICTIONARY]
design: docs/plans/2026-09-04-run-operations-requirements.md
---

# `source.convert` уходит из словаря: Unica читает одну выгрузку

**Решение.** Операция `source.convert` снимается со словаря `unica.run`.
Словарь не держит операций, которым нужен формат, не читаемый Unica: продукт
работает с выгрузкой Designer (Platform XML), EDT в нём не читается и не
пишется, и создание наборов в формате EDT требования откладывают не раньше
0.20. Наследника у `operation=convert` из v0.12 нет.

**Что чинит.** Требование A3 обосновывало операцию миграцией со старой
выгрузки — `check` советует `formatMigrationAvailable`, а выполнить совет
нечем. Но `v8-runner convert` переводит только Designer ↔ EDT через
`1cedtcli` и версию формата выгрузки Designer не меняет. Реализованная по
этому имени операция обслуживала бы нужду, которой у продукта нет, и вела
агента к отсутствующему EDT: опубликованное имя операции — адрес вызова, и
адрес в тупик хуже отсутствия адреса. Миграцию старой выгрузки делает путь
платформы — загрузка в базу и выгрузка заново текущим форматом, то есть
`infobase.configuration.load` и `source.dump`.

**Чем держится.** Проверка каталога перечисляет словарь целиком и падает на
имени с `convert` и на описании, обещающем EDT или «формат исходников».
Проверка состава словаря по-прежнему владеет точным списком; проза
`INV.APP.V13-RUN-DICTIONARY` перестаёт держать счёт «ровно двенадцать»,
который устарел ещё при `DEC.2026-09-09.PROJECT-CONFIG-IS-HANDWRITTEN`, —
состав словаря принадлежит проверке.

**Цена.** Словарь сходится с десяти операций до девяти, реализованных
остаётся четыре из девяти. Реализация `source.convert` над раннером
(превью, забор, пересчёт файлов в целях) остаётся в ветке
`feat/issue-871-a2-source-convert` без слияния: если EDT войдёт в продукт,
код и тесты на живых конвертах раннера 0.9.0 пригодятся, но опубликованного
обещания за ними нет.
