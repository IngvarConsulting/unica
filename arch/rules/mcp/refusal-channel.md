---
id: INV.WIRE.V13-REFUSAL-CHANNEL
check:
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
