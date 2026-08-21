---
id: INV.REGISTRY.SYMBOL-MATCHES-PATH
status: active
governs: process
decision: DEC.2026-08-18.REGISTRY-SHAPE
check: tests/arch/test_registry.py::test_symbol_matches_its_path
scope: [docs]
---

# Символ записи выводится из её пути
Символ записи и путь её файла выводятся друг из друга: читатель, у которого есть
символ, открывает файл без обращения к индексу.
