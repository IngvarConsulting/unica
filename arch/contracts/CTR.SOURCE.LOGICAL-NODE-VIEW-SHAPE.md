---
id: CTR.SOURCE.LOGICAL-NODE-VIEW-SHAPE
status: active
governs: product
decision: DEC.2026-08-25.LOGICAL-READ-CORE-SLICE
check: crates/unica-coder/src/domain/node_view.rs::node_view_contract_is_complete
scope: [product, source]
version: 1
producer: crates/unica-coder/src/domain/node_view.rs
consumers: [platform, review]
---

# Сериализованная форма скрытого логического узла v0.13

У каждого адресуемого узла ровно семь общих допустимых слотов: `at`, `kind`,
`title`, `props`, `branches`, `can`, `limits`. Пустые optional slots опускаются.
`items` появляется только у адресуемой collection/branch projection; строки
данных и строки исходника не получают `at`.

`ok`, `summary`, diagnostics, `rev` и `cursor` принадлежат `DomainResult`, а не
узлу. Узел не сериализует `set`, `sourceState`, `fileExists`, layout или raw
provider payload.
