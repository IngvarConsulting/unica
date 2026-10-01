---
id: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
check:
  - crates/unica-coder/src/domain/address.rs::qualified_addresses_are_table_driven_canonical_and_arbitrarily_deep
  - crates/unica-coder/src/domain/address.rs::qualified_addresses_reject_unqualified_malformed_and_noncanonical_roots
  - crates/unica-coder/src/domain/address.rs::metadata_aliases_reuse_v12_evidence_while_structural_aliases_stay_separate
  - crates/unica-coder/src/domain/address.rs::unqualified_input_resolves_only_with_one_source_set_and_stays_qualified
  - crates/unica-coder/src/domain/address.rs::configuration_kind_is_rejected_everywhere_except_the_sole_root
  - crates/unica-coder/src/domain/address.rs::nameless_kinds_do_not_consume_the_next_segment_as_a_name
  - crates/unica-coder/src/domain/address.rs::a_named_kind_still_takes_the_segment_that_follows_it
  - crates/unica-coder/src/domain/address.rs::neither_a_configuration_prefix_nor_an_invented_interface_name_is_accepted
---

# Логический адрес называет набор исходников и путь по видам объектов

Канонический адрес имеет вид `<набор>:<Вид>.<Имя>...` и всегда содержит
непустое имя набора исходников. Виды и прикладные имена могут чередоваться
на произвольной глубине. Русские псевдонимы видов приводятся к принятым
английским именам; написание прикладных имён сохраняется. Физического пути
в адресе нет.

Вид в конце без имени обозначает ветвь, например
`main:Catalog.Валюты.Attribute`. У `Configuration` и `Interface` имени
не бывает: следующий сегмент разбирается как вид. Выдуманное имя
отклоняется с объяснением этой причины. `Configuration` разрешён только
как самостоятельный корень `main:Configuration`.

Разрешение входного адреса в контексте проекта может подставить пропущенный
набор, только если доступен ровно один. Результат всё равно содержит набор;
строгий разбор канонического адреса его не подставляет.
