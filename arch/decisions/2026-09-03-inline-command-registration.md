---
id: DEC.2026-09-03.INLINE-COMMAND-REGISTRATION
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/v13_read/tests.rs::logical_reader_parity_contract_is_complete
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.LOGICAL-READER-PARITY]
design: docs/design/2026-09-03-inline-command-registration-design.md
---

# Команда владельца доказывается инлайн-регистрацией, а не файлом дескриптора

**Решение.** Регистрация дочернего объекта в `ChildObjects` владельца — это
текст элемента, а при его отсутствии `Properties/Name` вложенного
определения. Форма и макет остаются текстовой ссылкой с обязательным
matching descriptor. Команда регистрируется своим полным инлайн-определением
`<Command uuid="…"><Properties><Name>…`, файла `Commands/<Имя>.xml` не имеет,
и доказательство владельца для пары `Command.<Имя>` состоит из этой
регистрации; вложенных физических пар после команды нет. Модуль команды
читается по прежнему пути `Commands/<Имя>/Ext/CommandModule.bsl`, а
незарегистрированный каталог команды остаётся неадресуемым.

**Почему.** Так пишет платформа 8.3.27 и так описывает формат
`plugins/unica/references/specs/1c-config-objects-spec.md`; дамп УТ не
содержит ни одного дескриптора команды при 567 модулях команд, и любой
владелец с командами проваливал owner admission в `view` и `find`.

**Цена.** Прежние фикстуры v13 с отдельными дескрипторами команд заменены на
форму платформы; проверка неправильного дочернего дескриптора закреплена на
форме.
