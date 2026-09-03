---
id: CTR.WIRE.FIND-CANDIDATE-SHAPE
status: active
governs: product
version: 1
decision: DEC.2026-09-03.FIND-ADDRESS-PATH-DIRECTORY
producer: crates/unica-coder/src/application/v13/find.rs
consumers: [host]
check: crates/unica-coder/src/infrastructure/v13_find.rs::find_address_path_directory_contract_is_complete
scope: [wire]
---

# Кандидат find несёт адрес и место объекта в раскладке, но не ревизию

`data.candidates` содержит объекты с полями `at`, `kind`, `title`, `reason` и
`path`. `path` — место объекта в раскладке source set относительно его корня:
файл дескриптора, а у команды её каталог; поле опускается только у записи,
которую раскладка разместить не может. `nearest` появляется только при
отсутствии прямых совпадений.

Результат `find` не несёт `rev`: словарь соответствий адреса и файла не
является снимком ревизии, и клиент не может использовать его как ограду
для `apply`. Ограду по-прежнему выдаёт `view` или предпросмотр `apply`.
