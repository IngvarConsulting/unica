---
id: INV.SOURCE.SUBSYSTEM-REGISTRATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/subsystem_topology.rs::registered_topology_contract_is_complete
scope: [source]
---

# Топология подсистем выводится только из регистрации

Единый построитель под одним удерживаемым корнем, открытым без перехода по
символическим ссылкам, читает `Configuration.xml` и только транзитивно
зарегистрированные дескрипторы из `Configuration/ChildObjects` и
`Subsystem/ChildObjects`. Только они расходуют бюджеты и образуют зависимости
формата, незарегистрированная раскладка не влияет на доказательство, каждый
элемент `Content` имеет тип `MetadataAddress | UUID`, а каждый доказанный узел
принадлежит ровно одной эффективной роли.
