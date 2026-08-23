---
id: INV.WIRE.TYPED-READ-FINALIZER
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::typed_read_result_contract_is_closed
scope: [app, product, wire]
---

# Успешное типизированное чтение завершается только с data

Общий финализатор отклоняет успешное `Read + Typed` без
`OperationResult.data` как `typed_result_missing`, а текстовый дубль в stdout —
как `typed_result_textual`. Неуспешное чтение, мутация и внешний поток остаются
явно вне этого постусловия.
