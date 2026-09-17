---
id: INV.CACHE.OVERRIDE-PRIORITY
check:
  - crates/unica-bootstrap/src/host/runtime_cache.rs::the_explicit_override_outranks_every_host_source
---

# Явно заданный каталог установки важнее настроек хоста

Если `UNICA_RUNTIME_CACHE_DIR` содержит готовый путь, установщик Unica
выбирает его как корень каталога исполняемых компонентов (runtime).
Настройки `CLAUDE_PLUGIN_DATA`, `CODEX_HOME` и домашнего каталога пользователя
не заменяют этот путь и не добавляют к нему подкаталог.
