---
id: INV.PKG.RUNTIME-CACHE-RETENTION
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::collecting_keeps_the_newest_versions_of_each_artifact
  - crates/unica-bootstrap/tests/runtime_install.rs::collecting_leaves_an_artifact_that_is_within_the_limit
  - crates/unica-bootstrap/tests/runtime_install.rs::collecting_does_not_remove_a_version_owned_by_an_active_delivery_lock
---

# Очистка кеша сохраняет две свежие версии каждого артефакта

После успешной установки очистка выбирает для сохранения две последние готовые версии
каждого артефакта отдельно: ядро и движки не вытесняют друг друга.
Свежесть определяется временем маркера `.ready.json`, а не номером версии
или временем каталога. Версии, занятые активной блокировкой доставки,
сохраняются сверх лимита. Неудачное удаление также может оставить в кеше
больше двух версий.

Проверка установки использует кеш движка с маркерами готовности и загрузчик с тестовым
архивом ядра. Продолжение работающей сессии после удаления её движка
этими проверками не подтверждается.
