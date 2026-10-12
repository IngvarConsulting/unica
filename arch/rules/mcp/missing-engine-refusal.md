---
id: INV.WIRE.MISSING-ENGINE-REFUSAL
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::every_route_refuses_a_missing_engine_with_one_typed_refusal
  - crates/unica-coder/src/domain/engine.rs::the_missing_engine_refusal_carries_its_fields_in_data_and_one_code
  - crates/unica-coder/src/infrastructure/bundled_tools.rs::every_delivery_step_is_named_by_type_without_reading_text
  - crates/unica-coder/src/infrastructure/bundled_tools.rs::presence_skips_the_checksum_and_absence_on_the_failure_path_does_not
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::an_absent_bundled_runner_names_the_missing_engine_in_data
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::a_failed_engine_delivery_carries_the_missing_engine_fields
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::only_a_failed_launch_of_the_runner_is_a_missing_engine
  - crates/unica-coder/src/infrastructure/daemon/server.rs::module_check_answers_working_while_the_analyzer_is_being_delivered
  - crates/unica-coder/src/application/diagnostics.rs::absent_engine_reason_is_published_only_in_its_canonical_shape
  - crates/unica-coder/src/infrastructure/daemon/v13_client_run.rs::owned_launch_that_never_started_is_a_missing_engine
  - crates/unica-coder/src/infrastructure/platform/process.rs::only_a_refused_launch_is_a_launch_failure
  - crates/unica-coder/src/infrastructure/internal_adapters.rs::a_missing_working_directory_is_not_a_failed_launch
---

# Отсутствие поставляемого движка — один типизированный отказ

Если поставляемого движка нет, `unica.run` (раннер), `unica.search` с ролью
(поставщик поиска), `unica.check` модуля (анализ BSL) и `unica.view` ветви
графа вызовов `Caller` или `Callee` (анализатор) отвечают одним отказом:
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
переустановить плагин. Текст причины один на всех маршрутах, включая
причину неполного анализа `check`; ожидаемый путь едет только в `data.path`.

Шаг `start` получает только запуск, при котором процесс движка не возник:
неготовое дерево процессов или рабочий каталог и неудачное подключение уже
созданного процесса шагом `start` не считаются. Причина ОС остаётся
в сообщении: путь к движку из неё убран, секреты закрыты общей редакцией. Сбой после старта —
не отсутствие движка: он отвечает своим кодом без полей движка.

Неудачная доставка, после которой движка нет, отвечает тем же уточнением:
рядом с `data.delivery` идут поля движка на шаге `delivery`. Доставка,
которая ещё идёт, остаётся повторяемым `delivery_in_progress`.

Каждый маршрут сам заказывает доставку недостающего движка до проверки
наличия, поэтому повтор, который советует отказ, её и продолжает. `search`,
`check` и `view` после доставки проверяют только наличие файла, без хеша.
Контрольную сумму до запуска сверяют `run` и адаптер анализа BSL у `check`;
`search` и `view` запускают движок в службе рабочего пространства, и после
её отказа маршрут сам сверяет сумму, чтобы назвать шаг. Там же шаг `start`
устанавливается прямым запуском проверенного движка: решает отказ ОС
создать процесс, а не текст ошибки службы.
