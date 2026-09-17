---
id: INV.APP.EVENT-BINDING
check:
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::event_subscription_requires_explicit_non_global_module_fact
---

# Обработчик подписки требует явно неглобальный общий модуль

В описании общего модуля обработчика подписки должно быть указано
`Global=false`. Отсутствие свойства не считается значением `false`:
валидация подписки завершается ошибкой в поле `properties.handler`.
