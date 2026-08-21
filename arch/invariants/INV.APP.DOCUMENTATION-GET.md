---
id: INV.APP.DOCUMENTATION-GET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/documentation.rs::the_first_provider_owning_the_locator_answers_the_get
scope: [app]
---

# Полный документ отдаёт первый владелец локатора

Получение документа опрашивает поставщиков в порядке реестра и возвращает
ответ первого поставщика, который признал локатор своим.
