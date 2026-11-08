---
id: INV.APP.NATIVE-OPERATION-BACKEND
gap: https://github.com/IngvarConsulting/unica/issues/983
---

# Операции Unica исполняются без скриптового бэкенда

Продуктивный код `unica-coder`, `unica-coder-daemon`, `unica-application` и `unica-domain` не запускает `python`,
`python3`, `bash`, `powershell` или `pwsh` и не возвращается к файлам операций как запасному
исполнителю. Скрипты сборки и эталонные программы тестовых фикстур к этому
runtime не относятся.

Текущая проверка ищет буквальные вызовы `Command::new` с этими
интерпретаторами. Она не доказывает отсутствие запуска через другое имя или
переменную; этот пробел описан в `gap`.
