---
id: INV.APP.RECEIPT-TEST-SUPPORT-AUTHORITY
check:
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::facade_source_keeps_the_exact_abi_and_has_no_production_authority
  - crates/unica-coder/tests/daemon_receipt_ledger/facade_guards.rs::facade_and_non_owner_helpers_have_no_direct_receipt_ledger_authority
---

# Тестовая поддержка ReceiptLedger не получает полномочий владельца

Признак `receipt-ledger-test-support` открывает контракту ReceiptLedger
фасад `receipt_ledger_test_support` с закрытым набором публичных функций:
запуск сценария в бегунке контракта и вызов функций владельца, которые
вычисляют ключ квитанции, хеши и канонический терминал. Новая публичная
функция фасада — изменение этого обязательства, а не расширение обвязки.

Фасад не называет хранилище квитанций, его актор и порт, каталог
состояния демона, блокировку полномочий, удерживаемые полномочия каталогов
и файлов и прямой доступ к файловой системе. Строк производственных кодов
отказа и свидетельств в нём нет ни литералом, ни склейкой литералов через
`concat!` или форматирование. Фасад не включает файлов и не склеивает строки
через `.concat()`.
