---
id: INV.SOURCE.SNAPSHOT-BINDING
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/source_resources.rs
scope: [source]
---

# Ресурс действует внутри своего снимка и своей роли

Идентификатор ресурса имеет силу только в снимке, который его выдал, а право записи выдаётся
по доказанной роли ресурса, а не по совпадению пути.
