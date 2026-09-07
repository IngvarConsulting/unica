---
id: INV.SOURCE.TAIL-INSERT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/application/tool_contracts.rs::code_patch_tail_insert_public_contract_is_closed
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_without_a_selector_appends_to_the_end_and_proves_the_repeat
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_writes_the_first_body_of_an_empty_or_bom_only_module
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_creates_a_module_file_the_platform_never_exported
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_refuses_a_module_role_the_metadata_kind_never_owns
scope: [source]
---

# Вставка без селектора идёт в конец и доказывает повтор

Операция `insert` инструмента `unica.code.patch` принимает необязательный
`selector`. Без селектора содержимое дописывается в конец канонического
BSL-модуля, `position` не принимается, маркер порядка байтов содержимым не
считается и сохраняется, а повтор идентичного вызова до записи распознаётся как
семантически пустой. Отдельной операции инициализации нет. Отсутствующий файл
модуля создаётся только при применении записи и только когда роль допустима для
вида метаданных по реестру, а дескриптор владельца доказан; предпросмотр файл
не создаёт.
