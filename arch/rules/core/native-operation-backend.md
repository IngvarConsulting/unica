---
id: INV.APP.NATIVE-OPERATION-BACKEND
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_unica_coder_has_no_runtime_operation_script_fallback
gap: https://github.com/IngvarConsulting/unica/issues/983
---

# Операции Unica исполняются без скриптового бэкенда

Продуктивный код `unica-coder` не запускает `python`, `python3`, `bash`,
`powershell` или `pwsh` и не возвращается к файлам операций как запасному
исполнителю. Скрипты сборки и эталонные программы тестовых фикстур к этому
runtime не относятся.

Текущая проверка ищет известные классы старого бэкенда и буквальные вызовы
`Command::new`. Она не доказывает отсутствие запуска через другое имя или
переменную; этот пробел описан в `gap`.
