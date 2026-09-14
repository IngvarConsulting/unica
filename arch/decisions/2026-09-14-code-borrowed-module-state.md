---
id: DEC.2026-09-14.CODE-BORROWED-MODULE-STATE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/native_operations/code_module_state_tests.rs::borrowed_code_insert_and_replace_stage_module_state_atomically
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.CODE-BORROWED-MODULE-STATE]
---

# Каноническая запись BSL подключает заимствованный модуль

**Решение.** `code.insert` и `code.replace` включают недостающее состояние
модуля заимствованного объекта расширения в тот же staged state, что и BSL.
Состояние принадлежит ближайшему дескриптору модуля; у формы свойство
называется `Form`, у остальных модулей — платформенным именем роли.
Чистое преобразование XML общее с внутренним `cfe.patch_method`.

**Причина.** Успешная запись BSL оставляла неподключённый модуль.
Самостоятельная запись XML обошла бы забор ревизии и журнал публикации.
Дескриптор получает собственное событие изменения метаданных по
`INV.APP.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION`; транзакция подчиняется
`INV.CACHE.RETAINED-APPLY-REVISION-ROLLBACK` и
`INV.SOURCE.RETAINED-APPLY-WRITE-FREE`.

**Цена.** Даже совпадающий BSL может потребовать изменения XML, если состояние
отсутствует. Несовместимое состояние приводит к отказу до публикации.
Публичных операций и аргументов решение не добавляет.
