---
id: INV.SOURCE.SERVICE-SEQUENCES-ARE-BRANCHES
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::declared_service_kinds_finally_get_their_subject
---

# Методы и параметры служб читаются по отдельным адресам

Шаблоны URL HTTP-сервиса доступны через ветвь `URLTemplate`, операции
веб-сервиса — через `Operation`. Внутри них методы и параметры доступны
через `Method` и `Parameter`. Каждый элемент имеет собственный логический
адрес: один метод можно прочитать, не вычитывая список из строки свойств.
