---
id: INV.APP.DOCUMENTATION-GET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/documentation.rs::get_skips_a_non_owner_and_projects_the_owners_document
scope: [app]
---

# Get пропускает поставщика без ответа и проецирует документ владельца

После поставщика, вернувшего `None`, следующий владелец локатора возвращает
документ с проверенными полями происхождения, идентичности и полного текста.
