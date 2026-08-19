---
id: INV.PRODUCT.NO-FORMAT-MIGRATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_format_profile_contract.py
scope: [product]
---

# Unica не мигрирует формат выгрузки

Инструмент пишет в действующем профиле и не переводит источник между версиями формата:
миграция — работа платформы, а не наша.
