---
id: INV.PKG.CORE-FIRST-ACQUISITION
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::the_core_installs_without_any_engine_present
  - crates/unica-bootstrap/tests/runtime_install.rs::an_engine_is_installed_on_demand_under_its_own_version
gap: https://github.com/IngvarConsulting/unica/issues/952
---

# Установка ядра не загружает внешние движки

При обычном запуске загрузчик устанавливает только ядро `unica`, даже если
манифест описывает и внешние движки. Движок доставляется по отдельному запросу;
его установка также не требует загрузки ядра.

Предварительный прогрев всего кеша командой `prefetch` — отдельный режим.

Ядро завершает рукопожатие MCP и отдаёт полный каталог инструментов
без установленных внешних движков и без ожидания их загрузки. Связанные
проверки установки ещё не подтверждают эту границу на запущенном MCP.
