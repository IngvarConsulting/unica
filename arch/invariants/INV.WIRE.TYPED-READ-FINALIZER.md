---
id: INV.WIRE.TYPED-READ-FINALIZER
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/application/mod.rs::successful_typed_reader_without_data_fails_closed
  - crates/unica-coder/src/application/mod.rs::successful_typed_reader_with_stdout_duplicate_fails_closed
  - crates/unica-coder/src/application/mod.rs::failed_typed_reader_may_omit_data
  - crates/unica-coder/src/application/mod.rs::successful_typed_mutation_may_omit_data
scope: [app, product, wire]
---

# Успешное типизированное чтение завершается только с data

Общий финализатор отклоняет успешное `Read + Typed` без
`OperationResult.data` как `typed_result_missing`, а текстовый дубль в stdout —
как `typed_result_textual`. Неуспешное чтение, мутация и внешний поток остаются
явно вне этого постусловия.
