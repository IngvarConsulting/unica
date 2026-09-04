---
id: INV.SOURCE.IDEMPOTENT-ATTEMPT-METADATA
status: active
governs: product
decision: DEC.2026-08-21.MUTATION-IDEMPOTENCE-SCOPE
check:
  - crates/unica-coder/src/infrastructure/native_operations/interface.rs::repeated_interface_edit_preserves_identity_but_reports_attempted_update
  - crates/unica-coder/src/infrastructure/native_operations/mxl.rs::repeated_mxl_compile_preserves_identity_but_reports_attempted_update
scope: [source]
---

# Неизменная публикация может сохранить квитанцию о попытке

Повторные `unica.interface.edit` и `unica.mxl.compile` сохраняют байты и
идентичность файла, но каждый возвращает одну запись `changes` о предпринятом
обновлении; пустая физическая публикация не обещает пустую квитанцию этих двух
обработчиков.
