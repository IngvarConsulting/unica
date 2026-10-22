---
id: INV.PKG.RUNTIME-CACHE-RETENTION
check:
  - crates/unica-artifact-delivery/tests/runtime_install.rs::collecting_failure_does_not_fail_a_successful_installation
  - crates/unica-artifact-delivery/tests/runtime_install.rs::collecting_keeps_the_newest_versions_of_each_artifact
  - crates/unica-artifact-delivery/tests/runtime_install.rs::collecting_leaves_an_artifact_that_is_within_the_limit
  - crates/unica-artifact-delivery/tests/runtime_install.rs::collecting_does_not_remove_a_version_owned_by_an_active_delivery_lock
gap: https://github.com/IngvarConsulting/unica/issues/1747
---

# Очистка кеша сохраняет удерживаемые поставки и две свежие версии

После успешной установки очистка выбирает для сохранения две последние готовые версии
каждого артефакта отдельно: ядро и движки не вытесняют друг друга.
Этот лимит не разрешает удалить удерживаемую поставку.
Свежесть определяется временем маркера `.ready.json`, а не номером версии
или временем каталога. Сверх лимита сохраняются версии с активной
блокировкой доставки, [живыми потребителями](live-engine-retention.md)
или обязательством [читать удерживаемую историю](retained-history-reader.md).
Для последнего удерживается вся необходимая reader dependency closure,
а не только исполняемый файл. Неудачное удаление также может оставить в кеше
больше двух версий. Ошибка этой очистки не отменяет успешную установку.
Это не относится к очистке незавершённой транзакции установки.

Проверка установки использует кеш движка с маркерами готовности и загрузчик с тестовым
архивом ядра. Проверки подтверждают обычный отбор двух версий и защиту
активной доставки. Защита живых потребителей и cold-reader с зависимостями
ими не подтверждается; разрыв reader retention остаётся в `gap`,
разрыв живых потребителей — в [отдельном правиле](live-engine-retention.md).

Отказ очистки воспроизводится невозможностью открыть файл блокировки
старой версии. Проверка подтверждает успешную установку ядра при таком
отказе, но не моделирует блокировку исполняемого файла Windows.
