---
id: INV.RUNTIME.NO-REFUSAL-FALLBACK
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_skills.py::test_shipped_guidance_never_routes_runtime_refusal_through_fallbacks
scope: [product]
---

# Отказ runtime не превращается в скрытый запасной маршрут

Поставляемые скиллы и справка не предлагают `unica.build.*` или
`unica.runtime.job.*` как обход отказа `unica.runtime.execute`. Долговременное
задание остаётся отдельным явно выбранным workflow.
