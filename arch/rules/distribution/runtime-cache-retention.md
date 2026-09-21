---
id: INV.PKG.RUNTIME-CACHE-RETENTION
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::collecting_failure_does_not_fail_a_successful_installation
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
больше двух версий. Ошибка этой очистки не отменяет успешную установку.
Это не относится к очистке незавершённой транзакции установки.

Проверка установки использует кеш движка с маркерами готовности и загрузчик с тестовым
архивом ядра. [Сохранение версий для живых потребителей](live-engine-retention.md)
этими проверками не подтверждается и пока требует реализации.

Отказ очистки воспроизводится невозможностью открыть файл блокировки
старой версии. Проверка подтверждает успешную установку ядра при таком
отказе, но не моделирует блокировку исполняемого файла Windows.
