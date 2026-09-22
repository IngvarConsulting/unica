---
id: INV.APP.SOURCE-SELECTION-FOR-READ
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::view_find_admitted_snapshot_may_finish_after_map_change
---

# Допущенное чтение может завершиться после изменения карты исходников

Уже допущенные `view` и `resolve` завершают чтение по сохранённой карте,
даже если описание источников затем изменилось. Следующий вызов получает
новую карту и соответствующего ей актора.
