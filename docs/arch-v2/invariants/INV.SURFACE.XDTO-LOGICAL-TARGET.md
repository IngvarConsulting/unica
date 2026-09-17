---
id: INV.SURFACE.XDTO-LOGICAL-TARGET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_skills.py::test_xdto_skill_uses_one_confirmed_info_preview_apply_mcp_flow
scope: [wire]
---

# XDTO-сценарий выбирает пакет логическим адресом

Поставляемый XDTO-сценарий использует `sourceSet` и `metadataPath` и не передаёт
путь к `Package.bin`.
