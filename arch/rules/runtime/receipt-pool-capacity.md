---
id: INV.APP.RECEIPT-POOL-CAPACITY
check:
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::sixty_four_exact_entitlements_reopen_and_sixty_fifth_rejects_without_mutation
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::cancel_reserved_shares_the_live_count_without_reserving_result_bytes
gap: https://github.com/IngvarConsulting/unica/issues/985
---

# Переполнение квитанций отклоняет новый вызов, сохраняя принятые

Активные и неподтверждённые квитанции занимают общий пул до 64 записей
и 541065216 байт с учётом места, зарезервированного под результаты.
Нехватка места возвращает `receipt_capacity` до начала предметной работы.
Она не вытесняет незавершённые вызовы или ещё действующие результаты.

Отмена, пришедшая раньше вызова, занимает одну запись и до 1 КиБ,
но не резерв результата. У обычной принятой квитанции фактические байты
и оставшийся резерв вместе составляют 8 МиБ + 64 КиБ.

Проверки заполняют файловое хранилище, проверяют отказ, неизменность принятых
записей и повторное открытие. Независимая проверка численных границ и всех
состояний общего пула ещё требует дополнения, указанного в `gap`.
