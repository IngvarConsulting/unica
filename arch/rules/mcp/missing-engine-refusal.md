---
id: INV.WIRE.MISSING-ENGINE-REFUSAL
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::every_route_refuses_a_missing_engine_with_one_typed_refusal
  - crates/unica-coder/src/domain/engine.rs::the_missing_engine_refusal_carries_its_fields_in_data_and_one_code
  - crates/unica-coder/src/infrastructure/bundled_tools.rs::every_delivery_step_is_named_by_type_without_reading_text
  - crates/unica-coder/src/infrastructure/bundled_tools.rs::presence_skips_the_checksum_and_absence_on_the_failure_path_does_not
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::an_absent_bundled_runner_names_the_missing_engine_in_data
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::a_failed_engine_delivery_carries_the_missing_engine_fields
---

# Отсутствие поставляемого движка — один типизированный отказ

Если поставляемого движка нет, `unica.run` (раннер), `unica.search` с ролью
(поставщик поиска), `unica.check` модуля (анализ BSL) и `unica.view` со
сводкой `callGraph` или ветвью графа (анализатор) отвечают одним отказом:
`provider_unavailable` с уточнением `provider_absent`. В `data` отказ несёт
`tool`, `path`, `target`, `pinnedVersion`, `stage` и `retryable: false`.
Неизвестный путь или пин передаётся `null`.

`stage` называет шаг поставки, на котором движка не оказалось: `manifest` —
манифест не найден, не читается или не называет движок; `target` — движок
не упакован под эту платформу; `delivery` — файла нет или его доставка не
удалась; `checksum` — контрольная сумма файла не совпала с манифестом;
`start` — проверенный движок не запустился. Шаг определяется типом отказа
резолвера, а не текстом ошибки. Сообщение называет следующий шаг для режима
установки: собрать движок в исходном checkout, дождаться доставки или
переустановить плагин.

Неудачная доставка, после которой движка нет, отвечает тем же уточнением:
рядом с `data.delivery` идут поля движка на шаге `delivery`. Доставка,
которая ещё идёт, остаётся повторяемым `delivery_in_progress`.

Наличие файла проверяется до запуска поставщика без хеша; контрольная
сумма сверяется на пути отказа, когда запуск не удался.
