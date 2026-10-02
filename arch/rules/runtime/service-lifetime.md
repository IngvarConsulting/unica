---
id: INV.RUNTIME.SERVICE-LIFETIME
check: []
gap: https://github.com/IngvarConsulting/unica/issues/978
---

# Внутренний сервис не остаётся запущенным без ограничения срока

По умолчанию workspace helper завершает работу после 7200 секунд простоя
или 28800 секунд с запуска. Значения меняют переменные
`UNICA_WORKSPACE_SERVICE_IDLE_SECS` и `UNICA_WORKSPACE_SERVICE_MAX_AGE_SECS`.
Остановка проходит через отмену зарегистрированной работы.

Проверка чтения настроек существует; исполнение этих границ требует
проверки жизненного цикла сервиса.
