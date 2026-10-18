---
id: INV.WIRE.RESOLVE-EXACT-BRIDGE
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - crates/unica-coder/src/application/v13/resolve.rs::resolve_takes_exactly_one_side_of_the_bridge
---

# Запрос resolve выбирает одну сторону адресного моста

`unica.resolve` принимает ровно один аргумент: логический адрес `at`
или путь `path`. Два аргумента вместе и отсутствие обоих дают `bad_value`.

Успешный ответ связывает существующий предмет с его логическим адресом и
[действительным расположением](resolved-source-location.md). Поиск по пути
соблюдает [точечное доказательство цели](../workspace/sources/source-path-resolution.md),
а формы и модули — [идентичность конкретного ресурса](../workspace/sources/resolved-resource-identity.md).
