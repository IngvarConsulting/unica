---
id: INV.SOURCE.DECLARATION-BRANCHES-FOLLOW-THE-NAME
status: active
governs: product
decision: DEC.2026-09-10.ROOT-DECLARATIONS-GET-BRANCHES
check: crates/unica-coder/src/infrastructure/v13_read/tests.rs::root_declarations_get_branches_and_only_the_named_one_is_addressable
scope: [product, source]
---

# Адресуется декларация с именем, а безымянная остаётся строкой

Стандартная табличная часть адресуется по имени и несёт вложенную ветвь своих
стандартных реквизитов. Характеристика прикладного имени не носит, поэтому
приходит строкой данных без адреса, а обращение к ней по имени отказывает.
