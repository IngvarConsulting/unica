---
id: INV.SOURCE.SUBSYSTEM-ADDRESS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::public_subsystem_info_registration_address_and_schema_contract_is_complete
scope: [source]
---

# Адрес подсистемы следует диалекту БСП

Публично зарегистрированный `subsystem.info` принимает логический адрес
`Subsystem.Parent`, возвращает зарегистрированное дерево имён и не публикует в
типизированных данных физические `Subsystems/`, обратную косую черту или `.xml`.
