---
id: INV.SOURCE.SNAPSHOT-BINDING
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_resources.rs::source_resources_ids_are_valid_only_inside_the_snapshot_that_issued_them
scope: [source]
---

# Ресурс действует только внутри выдавшего его снимка

Идентификатор ресурса нельзя прочитать с идентификатором другого снимка, даже
если оба снимка получены от одного поставщика для одного источника.
