---
id: INV.PKG.ATTRIBUTION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_attributions.py::test_repository_attribution_page_is_complete_and_linked
scope: [pkg]
---

# Атрибуция остаётся полной

Каждое заимствование названо в поставляемом файле атрибуции, и полнота проверяется против
реестра происхождения.
