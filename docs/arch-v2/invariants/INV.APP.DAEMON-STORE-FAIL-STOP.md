---
id: INV.APP.DAEMON-STORE-FAIL-STOP
---

# Ограничения файлового хранилища и публикации записи

File store ограничивает writer acquisition, размер record, recovery enumeration
и число retained records. Create использует preallocated TaskId и атомарную
публикацию без замены; collision типизирован. Успешный rename изменяет in-memory retention catalog до fallible directory
sync, поэтому видимый uncertain record учитывается немедленно и после reopen.
Pre-rename failure catalog не меняет. Record ограничен 8 MiB canonical result
плюс 64 KiB envelope; serialization использует исходный store deadline.
