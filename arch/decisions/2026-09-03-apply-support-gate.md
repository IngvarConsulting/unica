---
id: DEC.2026-09-03.APPLY-SUPPORT-GATE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::locked_vendor_objects_refuse_every_family_before_staging
supersedes: []
superseded-by: null
establishes: [INV.SAFETY.APPLY-SUPPORT-GATE]
design: docs/design/2026-09-03-apply-support-gate-design.md
---

# Канонический `apply` держит страж поддержки поставщика

**Решение.** Планировщик `unica.apply` проверяет правило поддержки владельца
адресованного узла до того, как любое семейство операций положит первый байт
в staged state. Владелец — объект метаданных верхнего уровня адреса или
корень конфигурации. При политике `deny` конфигурация на поддержке с
запрещёнными изменениями отвечает `invalid_state` для любого объекта, а объект
поставщика с правилом «не редактируется» — `invalid_state` с подсказкой
`supportRule.set`. Закрытое исключение — операции самой поддержки
(`supportCapability.set`, `supportRule.set`): они и есть выход из замка.

**Что чинит.** Старый диспетчер требовал редактируемого владельца для каждого
мутатора Platform XML. На каноническом пути страж держали только семейства
`code` и `xdto`; `metadata`, `form_resource` и `dcs_mxl` публиковали правки
запертого объекта. Это касалось и снятого в первой партии `form.add`.

**Проверки.** Тест шва планировщика гоняет пять семейств на запертом объекте,
операцию поддержки, запрет изменений и отсутствие маркера; корпус приёмки
замораживает те же ответы на выделенном объекте поставщика
`Enum.НастройкиПоставщика` (S281–S288).
