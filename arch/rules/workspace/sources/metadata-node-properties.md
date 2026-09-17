---
id: INV.SOURCE.METADATA-NODE-PROPERTIES
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::metadata_node_props_carry_the_observed_object_properties
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::metadata_node_props_lay_out_the_per_kind_facts_by_role
---

# Свойства объекта доступны в его логическом узле

`props` узла метаданных содержит наблюдаемые свойства под ключами из
закрытого словаря профиля вида. Например, справочник публикует
`Hierarchical` и `CodeLength` под этими именами.

Составной факт с отдельными ролями раскладывается на плоские поля:
обработчик задания — `handlerModule` и `handlerMethod`, расписание регистра
расчёта — `scheduleRegister`, `scheduleValueField` и `scheduleDateField`.
Остальные составные значения, например тип константы, приходят компактной
строкой, если помещаются в ограничение размера `props`. Вложенные объекты
и массивы в `props` не публикуются.
