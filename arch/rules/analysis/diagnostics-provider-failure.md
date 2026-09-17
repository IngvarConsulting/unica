---
id: INV.APP.DIAGNOSTIC-PROVIDERS
check:
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_concurrency_contains_provider_panic_and_keeps_sibling_items
---

# Сбой одного диагностического движка не стирает чужие результаты

Если один из параллельно опрашиваемых поставщиков диагностики завершается
паникой, его секция получает статус `Failed` и код `provider_panicked`.
Находки исправного поставщика сохраняют его имя, код и текст.
Общий результат помечается как частичный.
