---
id: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
status: active
governs: product
decision: DEC.2026-09-07.NAMELESS-KINDS-IN-ADDRESS
check:
  - crates/unica-coder/src/domain/address.rs::qualified_addresses_are_table_driven_canonical_and_arbitrarily_deep
  - crates/unica-coder/src/domain/address.rs::qualified_addresses_reject_unqualified_malformed_and_noncanonical_roots
  - crates/unica-coder/src/domain/address.rs::metadata_aliases_reuse_v12_evidence_while_structural_aliases_stay_separate
  - crates/unica-coder/src/domain/address.rs::unqualified_input_resolves_only_with_one_source_set_and_stays_qualified
  - crates/unica-coder/src/domain/address.rs::configuration_kind_is_rejected_everywhere_except_the_sole_root
  - crates/unica-coder/src/domain/address.rs::nameless_kinds_do_not_consume_the_next_segment_as_a_name
  - crates/unica-coder/src/domain/address.rs::a_named_kind_still_takes_the_segment_that_follows_it
scope: [product, source]
version: 2
producer: crates/unica-coder/src/domain/address.rs
consumers: [platform, review]
---

# Квалифицированный адрес скрытого логического дерева v0.13

Каноническая строка результата имеет форму
`<sourceSet>:<Kind>[.<Name>...]`, всегда содержит непустой набор исходников и
чередует вид с прикладным именем, кроме видов, которые имени не носят. Контекстный resolver принимает вход без
`sourceSet:` только при единственном доступном наборе и возвращает
квалифицированный адрес; строгий parser identity префикс не выводит. Последний
вид вправе не иметь имени — так адресуется ветка метаданных. Отдельно от этого
вид может не носить имени вовсе: он единственен у своего владельца, и тогда
следующий сегмент читается как вид, а не как имя. Таких видов два —
`Configuration`, единственное представление корня конфигурации, запрещённое на
любой другой позиции, и `Interface`, командный интерфейс владельца. Сегмент
после безымянного вида, не читающийся как вид, отклоняется с указанием, что
имени у этого вида нет. Русские псевдонимы видов нормализуются в
доказанные английские токены, прикладные имена сохраняются. Физический путь в
строку не входит.
