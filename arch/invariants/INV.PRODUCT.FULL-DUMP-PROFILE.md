---
id: INV.PRODUCT.FULL-DUMP-PROFILE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_format_profile_contract.py::test_full_dump_uses_the_shared_active_profile
scope: [product]
---

# Полная выгрузка использует общий активный профиль

Публикация полной выгрузки берёт строку платформы и версию формата из общего
`ACTIVE_FORMAT_PROFILE`, не закрепляя рядом собственные литералы 8.3.27/2.20.
