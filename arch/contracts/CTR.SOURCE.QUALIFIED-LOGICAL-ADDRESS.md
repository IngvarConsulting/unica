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

Каноническая строка результата имеет форму
`<sourceSet>:<Kind>[.<Name>...]`, всегда содержит непустой набор исходников и
чередует вид с прикладным именем. Контекстный resolver принимает вход без
`sourceSet:` только при единственном доступном наборе и возвращает
квалифицированный адрес; строгий parser identity префикс не выводит. Последний
вид вправе не иметь имени. `Configuration` является единственным
представлением корня конфигурации и запрещён на любой другой позиции; вид без
имени представляет ветку метаданных. Русские псевдонимы видов нормализуются в
доказанные английские токены, прикладные имена сохраняются. Физический путь в
строку не входит.
