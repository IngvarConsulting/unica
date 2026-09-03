---
id: INV.SOURCE.IDEMPOTENT-ATTEMPT-METADATA
status: active
governs: product
decision: DEC.2026-08-21.MUTATION-IDEMPOTENCE-SCOPE
check: crates/unica-coder/src/infrastructure/native_operations/source_invariant_tests.rs::repeated_interface_and_mxl_mutations_preserve_file_identity_but_report_attempted_updates
scope: [source]
---

# Неизменная публикация может сохранить квитанцию о попытке

Повторные `unica.interface.edit` и `unica.mxl.compile` сохраняют байты и
идентичность файла, но каждый возвращает одну запись `changes` о предпринятом
обновлении; пустая физическая публикация не обещает пустую квитанцию этих двух
обработчиков.
