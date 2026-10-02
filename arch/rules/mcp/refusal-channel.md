---
id: INV.WIRE.V13-REFUSAL-CHANNEL
check:
  - crates/unica-coder/src/domain/invocation.rs::refusal_envelope::a_refusal_carries_code_outcome_and_message_and_nothing_else
  - crates/unica-coder/src/domain/invocation.rs::refusal_envelope::a_detail_overrides_the_default_outcome_of_its_code
  - crates/unica-coder/src/domain/invocation.rs::refusal_envelope::every_code_answers_with_one_of_the_six_outcomes
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_refusals_answer_one_diagnostics_channel_from_the_closed_code_set
---

# Отказ передаётся через diagnostics и отличает устаревшую ревизию

Канонический вызов передаёт отказ через `diagnostics`: первый элемент
содержит закрытый код и непустое сообщение. Второго кода в `data.code`
нет. Конфликт `ifRev` получает `stale_revision` с ожидаемой и фактически
допущенной ревизиями.

Отсутствие файлового поддерева у допущенного логического scope поиска
даёт успешный пустой результат без диагностик. Это не отказ провайдера
и не сырая ошибка файловой системы.

Проверка исполняет предметные вызовы через production daemon runtime;
форму compatibility Task-ответов задаёт отдельный контракт.

Диагностика отказа содержит `outcome`: повторить вызов (`retryAsIs`), исправить
аргументы (`fixCall`), восстановить исходники (`fixSource`), обратиться
к человеку (`needsHuman`), выбрать другой маршрут (`goElsewhere`) или
остановиться (`deadEnd`). Исход определяется кодом; уточнение `detailCode`
может выбирать его у `provider_unavailable` и `task_backend_failed`.
Без уточнения эти два кода требуют человека. Поле `detailCode` без значения
не добавляется.

`fixSource` не разрешает автоматически доделывать повреждённый сторонней
правкой объект: применяется [правило восстановления](../workspace/sources/damaged-object-recovery.md).
Проверки формы и выбора исхода проходят конструктор предметного ответа;
они не доказывают правильность классификации каждого места отказа.

Неподдерживаемый вариант операции или фильтра отвечает подходящим
`unsupported_*`, а не маскируется под сбой поставщика. Это не отменяет
отдельно обозначенные пробелы поставщика: например, чтение модуля
[`WebSocketClient`](../workspace/sources/module-capabilities.md).
