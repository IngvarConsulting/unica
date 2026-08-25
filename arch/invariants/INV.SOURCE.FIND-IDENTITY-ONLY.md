---
id: INV.SOURCE.FIND-IDENTITY-ONLY
status: active
governs: product
decision: DEC.2026-08-25.LOGICAL-READ-CORE-SLICE
check: crates/unica-coder/src/infrastructure/v13_find.rs::find_identity_facts_and_results_are_independent_of_bsl_body_bytes
scope: [app, product, source]
---

# Find индексирует identity facts и не читает содержимое исходника

Индекс строится из actor-owned retained Platform XML roots с ограничениями на
число source sets, документов, суммарные fact bytes и размер descriptor reads. Его факты —
квалифицированный address, канонический kind, programmatic name, локализованные
synonyms и export path.

Два дерева с одинаковыми metadata descriptors и разными BSL/body bytes дают
byte-equivalent facts и одинаковые exact/nearest results. Nearest использует
только `name` или `address`; content/body/symbol reason не существует.
