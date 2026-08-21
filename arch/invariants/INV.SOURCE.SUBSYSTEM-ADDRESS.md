---
id: INV.SOURCE.SUBSYSTEM-ADDRESS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/subsystem.rs::registered_names_build_the_same_public_address
scope: [source]
---

# Адрес подсистемы следует диалекту БСП

`subsystem.info` публикует только адреса `SubsystemAddress` в диалекте БСП,
выведенные из зарегистрированных имён под доказанным корнем, без физических
токенов вида метаданных.
