---
id: INV.CACHE.OVERRIDE-PRIORITY
---

# Явно заданный каталог установки важнее настроек хоста

Если `UNICA_RUNTIME_CACHE_DIR` содержит готовый путь, установщик Unica
выбирает его как корень каталога исполняемых компонентов (runtime).
Настройки `CLAUDE_PLUGIN_DATA`, `ZCODE_PLUGIN_DATA`, `CODEX_HOME` и домашнего
каталога пользователя не заменяют этот путь и не добавляют к нему подкаталог.

Если готового пути нет, каталог выбирается в порядке:
`<CLAUDE_PLUGIN_DATA>/runtimes`, `<ZCODE_PLUGIN_DATA>/runtimes`,
`<CODEX_HOME>/unica/runtimes`, `<HOME>/.codex/unica/runtimes`, затем
`<USERPROFILE>/.codex/unica/runtimes`. При совпадающих переменных данных
Claude и ZCode результат тот же; при разных сохраняется приоритет Claude.
Пустой или нераскрытый `ZCODE_PLUGIN_DATA` не задаёт каталог.
Значение override с нераскрытым `${` пропускается: каталог с буквальным
шаблоном не создаётся. Без пригодного значения из этой цепочки установка
отказывает.
