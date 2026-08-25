---
id: INV.SOURCE.FIND-IDENTITY-ONLY
status: active
governs: product
decision: DEC.2026-08-25.LOGICAL-READ-CORE-SLICE
check: crates/unica-coder/src/infrastructure/v13_find.rs::find_identity_only_contract_is_complete
scope: [app, product, source]
---

# Find индексирует identity facts и исключает исполняемое тело исходника

Индекс проходит адресуемое typed logical tree из actor-owned retained Platform
XML roots с одной aggregate logical-read deadline, cancellation и exact revision fence, а
также с ограничениями на число source sets, документов, суммарные fact bytes и
размер descriptor reads. Существующий malformed или wrong-owner descriptor
даёт `provider_unavailable`. Факты — qualified address, canonical kind,
programmatic name, локализованные synonyms и export path. Method и region
identities разрешено извлекать bounded-разбором декларативной оболочки BSL;
`Body`, statements и строки исходника не являются nodes/facts и BFS их не
читает повторно. В пределах одной операции exact module projection и owner
inventory читаются один раз на actor identity/revision/address; новый actor или
revision не наследует cached evidence.

Два дерева с одинаковыми metadata descriptors и одинаковыми
declarations/regions, но разными statements, дают byte-equivalent facts и
одинаковые exact/nearest results. Все profile-permitted module identities
зарегистрированного owner достижимы через его единственную `Module` branch;
отсутствующий module source не скрывает parent-projected identity. Nearest
использует только `name` или `address`; content/body/symbol reason не существует.
Operation budget равен 120 секундам, превышает семисекундный Task handoff и не
отменяет callback при handoff.
