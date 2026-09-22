---
id: INV.APP.EVENT-BINDING
check:
  - crates/unica-coder/src/domain/metadata/event_subscription.rs::event_subscription_binding_requires_exact_case_presence_and_one_signature
  - crates/unica-coder/src/domain/metadata/event_subscription.rs::event_subscription_binding_validates_the_complete_handler_contract
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::event_subscription_requires_explicit_non_global_module_fact
gap: https://github.com/IngvarConsulting/unica/issues/931
---

# Источники, событие и обработчик подписки должны быть совместимы

Валидатор подписки требует непустой набор источников. Событие с точным именем
и регистром должно существовать у каждого источника с одинаковой сигнатурой.
Одного совпадения имени события недостаточно.

Обработчик разрешается в том же владельце конфигурации или расширения,
что и подписка. Одноимённый модуль другого набора исходников не подставляется.

Обработчиком служит экспортная процедура общего модуля с явно заданными
`Global=false` и `Server=true`. Число её параметров равно числу параметров
события плюс один для источника. Отсутствующий флаг не заменяется
подходящим значением по умолчанию; имена параметров не проверяются.

Эти проверки доказывают работу валидатора. Его обязательное применение
к итоговому объекту относится к [корректности изменения](metadata-mutation-validity.md).

Изоляция при разрешении обработчика требует отдельной проверки текущего `apply`;
тесты валидатора принимают уже подготовленные сведения о модуле.
