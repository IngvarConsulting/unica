---
id: INV.APP.DAEMON-TASK-PERSISTENCE
---

# Ограничения содержимого и размера сохранённого результата

Raw arguments, caller/runtime text, stdout, stderr и свободный failure text
в record не попадают. Ошибка Task строится только из закрытой причины при чтении.

Persistent envelope имеет отдельный запас 64 KiB сверх результата.
