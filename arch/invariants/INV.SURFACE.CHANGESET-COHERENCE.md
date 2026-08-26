---
id: INV.SURFACE.CHANGESET-COHERENCE
status: active
governs: process
decision: DEC.2026-08-18.REGISTRY-SHAPE
check: tests/arch/test_product_immutability.py::test_surface_ledger_change_without_new_product_ground_is_caught
scope: [docs, wire]
---

# Изменение ведомости требует нового продуктового основания

После принятия v2 изменение порождённой `arch/tool-surface.md` допускается
только вместе с новым active product decision, который явно называет
`changes: [CTR.WIRE.TOOL-SURFACE]`, и принадлежащим ему продуктовым
wire-инвариантом или контрактом. Любое другое wire-решение основанием не
считается. Паритет ведомости с живым реестром проверяется отдельно
`CTR.WIRE.TOOL-SURFACE`.
