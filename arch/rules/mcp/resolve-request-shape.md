---
id: INV.WIRE.RESOLVE-EXACT-BRIDGE
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - crates/unica-coder/src/application/v13/resolve.rs::resolve_takes_exactly_one_side_of_the_bridge
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_paths_is_exclusive_with_at_and_path_and_takes_only_strings
---

# Запрос resolve выбирает одну сторону адресного моста

`unica.resolve` принимает ровно один аргумент: логический адрес `at`,
путь `path` или [пакет путей](resolve-path-batch.md) `paths`. Несколько
из них вместе и отсутствие всех дают `bad_value`.

Успешный ответ связывает существующий предмет с его логическим адресом и
[действительным расположением](resolved-source-location.md). Поиск по пути
соблюдает [точечное доказательство цели](../workspace/sources/source-path-resolution.md),
а формы и модули — [идентичность конкретного ресурса](../workspace/sources/resolved-resource-identity.md).
