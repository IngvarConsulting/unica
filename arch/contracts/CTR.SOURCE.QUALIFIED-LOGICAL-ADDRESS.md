---
id: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
status: active
governs: product
decision: DEC.2026-08-25.LOGICAL-TREE-CORE-SLICE
check: crates/unica-coder/src/domain/address.rs::qualified_logical_address_contract_is_complete
scope: [product, source]
version: 1
producer: crates/unica-coder/src/domain/address.rs
consumers: [platform, review]
---

# Квалифицированный адрес скрытого логического дерева v0.13

Каноническая строка имеет форму `<sourceSet>:<Kind>[.<Name>...]`, всегда
содержит непустой набор исходников и чередует вид с прикладным именем. Последний
вид вправе не иметь имени. `Configuration` является единственным представлением
корня конфигурации; вид без имени представляет ветку метаданных. Русские
псевдонимы видов нормализуются в доказанные английские токены, прикладные имена
сохраняются. Физический путь в строку не входит.
