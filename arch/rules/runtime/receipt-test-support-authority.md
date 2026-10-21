---
id: INV.APP.RECEIPT-TEST-SUPPORT-AUTHORITY
check:
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::facade_source_keeps_the_exact_abi_and_has_no_production_authority
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::facade_and_non_owner_helpers_have_no_direct_receipt_ledger_authority
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::facade_authority_guard_scans_every_named_production_source
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::rust_authority_guard_finds_store_identifiers_inside_formatting_macros
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::facade_authority_guard_fails_closed_on_dynamic_string_construction
---

# Тестовая поддержка ReceiptLedger не получает полномочий владельца

Признак `receipt-ledger-test-support` открывает контракту ReceiptLedger
закрытый фасад `receipt_ledger_test_support`. Через него можно запустить
сценарий в производственном бегунке и вычислить идентичности по замороженным
векторам владельца. Новый вход фасада — изменение этого обязательства, а не
расширение тестовой обвязки.

Фасад и вспомогательный код, не владеющий хранилищем, не называют хранилище
квитанций, его актор и порт, блокировку полномочий, retained-полномочия
каталогов и файлов и прямой доступ к файловой системе. Строки
производственных кодов отказа и свидетельств они не составляют ни
литералом, ни склейкой. Переходы квитанции делает только её владелец
по [правилу одного владельца](execution-authority.md).

Страж разбирает синтаксис исходника. Если он не может доказать, что строка
не собирается динамически, он отказывает, а не пропускает.
