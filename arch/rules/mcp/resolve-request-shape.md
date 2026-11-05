---
id: INV.WIRE.RESOLVE-EXACT-BRIDGE
---

# Запрос resolve выбирает одну сторону адресного моста

`unica.resolve` принимает ровно один аргумент: логический адрес `at`,
путь `path` или [пакет путей](resolve-path-batch.md) `paths`. Несколько
из них вместе и отсутствие всех дают `bad_value`.

Успешный ответ связывает существующий предмет с его логическим адресом и
[действительным расположением](resolved-source-location.md). Поиск по пути
соблюдает [точечное доказательство цели](../workspace/sources/source-path-resolution.md),
а формы и модули — [идентичность конкретного ресурса](../workspace/sources/resolved-resource-identity.md).
